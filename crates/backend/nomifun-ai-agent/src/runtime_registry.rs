//! Process-local registry for live Agent runtimes.
//!
//! A registry entry is keyed by conversation ID and owns the long-lived Agent
//! process/session reused across turns. It is not a persisted execution step or
//! DAG task. Turn admission is handled separately by Conversation's
//! `AgentTurnHandle`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use dashmap::DashMap;
use futures_util::future::BoxFuture;
use nomi_agent::session::SessionManager;
use nomifun_common::{
    AgentKillReason, AgentType, AppError, ConversationStatus, ErrorChain, OnConversationDelete,
    ProviderWithModel, TimestampMs, now_ms,
};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::nomi_session_persistence::{NomiSessionPersistence, NomiSessionResetOutcome};
use crate::runtime_handle::AgentRuntimeHandle;
use crate::types::AgentRuntimeBuildOptions;

/// Factory function that creates an [`AgentRuntimeHandle`] from build options.
///
/// Async so the factory can do real I/O (spawn a CLI process, negotiate the
/// ACP initialize handshake, etc.) without needing to `block_on` inside the
/// `AgentRuntimeRegistry` call site. Returning `BoxFuture` keeps the trait
/// object-safe for DI.
pub type AgentRuntimeFactory =
    Arc<dyn Fn(AgentRuntimeBuildOptions) -> BoxFuture<'static, Result<AgentRuntimeHandle, AppError>> + Send + Sync>;

/// Non-secret identity of the exact provider invocation graph a long-lived
/// Nomi runtime was built from. The provider-level revision changes whenever
/// credentials, connection roots or task capabilities change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeModelConfigBinding {
    pub provider_id: String,
    pub model: String,
    pub config_revision: i64,
}

/// Resolve the current exact Chat binding for one selected Nomi model. Product
/// composition supplies the ModelInvoke-backed implementation; keeping the
/// closure async lets the registry validate every admission under its existing
/// per-conversation lifecycle gate.
pub type AgentRuntimeModelConfigResolver = Arc<
    dyn Fn(ProviderWithModel) -> BoxFuture<'static, Result<RuntimeModelConfigBinding, AppError>>
        + Send
        + Sync,
>;

/// Manages the lifecycle of active per-conversation Agent runtimes.
///
/// Each conversation has at most one live runtime. Concurrent creation is
/// single-flight and every returned [`AgentRuntimeHandle`] references that same
/// runtime until it is terminated or evicted.
/// The trait is object-safe for dependency injection.
#[async_trait]
pub trait AgentRuntimeRegistry: Send + Sync {
    /// Get an existing runtime by conversation ID.
    fn get_runtime(&self, conversation_id: &str) -> Option<AgentRuntimeHandle>;

    /// Get an existing runtime or create one if none exists.
    ///
    /// Concurrent callers with the same `conversation_id` block on a shared
    /// [`OnceCell`] so the factory runs at most once per conversation —
    /// avoiding the race where two concurrent HTTP requests (e.g.
    /// `/messages` + `/warmup`) would each spawn their own CLI process and
    /// ACP connection, with one of them leaking.
    async fn get_or_create_runtime(
        &self,
        conversation_id: &str,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError>;

    /// Preparation-only runtime acquisition used by view warmup and other
    /// background preflight work.
    ///
    /// This may build or reuse a runtime slot, but it must never acquire,
    /// replace, or otherwise mutate the active turn-generation admission.
    /// Cancellation only fences this preparation request.
    async fn get_or_create_runtime_for_preparation(
        &self,
        conversation_id: &str,
        cancellation: CancellationToken,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(AppError::Conflict(format!(
                "Agent runtime preparation for conversation {conversation_id} was cancelled"
            ))),
            result = self.get_or_create_runtime(conversation_id, options) => result,
        }
    }

    /// Turn-generation-aware build used by Conversation sends. The default
    /// preserves compatibility for test/custom registries; the in-memory
    /// production registry overrides it so a stop that lands before/during a
    /// cold factory build is a durable tombstone for that exact generation.
    async fn get_or_create_runtime_for_turn(
        &self,
        conversation_id: &str,
        turn_generation: u64,
        cancellation: CancellationToken,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        let _ = turn_generation;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(AppError::Conflict(format!(
                "Agent runtime build for conversation {conversation_id} was cancelled"
            ))),
            result = self.get_or_create_runtime(conversation_id, options) => result,
        }
    }

    /// Release one exact turn-generation admission without tearing down the
    /// reusable runtime slot.
    ///
    /// Production callers invoke this only after durable/local turn ownership
    /// has reached a terminal boundary. A stale generation is an absorbing
    /// no-op and must never clear a successor admission.
    async fn release_runtime_turn(
        &self,
        conversation_id: &str,
        turn_generation: u64,
    ) -> Result<(), AppError> {
        let _ = (conversation_id, turn_generation);
        Ok(())
    }

    /// Tombstone and terminate one Conversation turn generation. A newer turn
    /// uses a different generation and can never be hit by this cancellation.
    fn cancel_runtime_turn(
        &self,
        conversation_id: &str,
        turn_generation: u64,
        reason: Option<AgentKillReason>,
    ) -> Result<(), AppError> {
        // A registry that does not track generation-to-runtime identity cannot
        // safely translate this into conversation-wide termination.
        let _ = (conversation_id, turn_generation, reason);
        Ok(())
    }

    /// Terminate and remove a runtime.
    fn terminate(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError>;

    /// Result-bearing awaitable teardown. Production registries override this
    /// so callers can distinguish a proven process exit from a failed kill.
    ///
    /// Registries that cannot prove process-tree exit must fail closed:
    /// treating an unverified teardown as success would allow a caller to
    /// finalize and reopen a durable Running Conversation while its old
    /// process could still execute.
    fn terminate_and_wait_result(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        let conversation_id = conversation_id.to_owned();
        let _ = reason;
        Box::pin(async move {
            Err(AppError::Internal(format!(
                "Agent runtime registry cannot prove teardown for conversation {conversation_id}"
            )))
        })
    }

    /// Clear the exact current-generation Nomi transcript after its runtime has
    /// reached a proven exit.
    ///
    /// Implementations must serialize this with runtime factory admission. An
    /// unsupported/unconfigured registry fails closed instead of pretending
    /// the resumable transcript was erased.
    fn reset_persisted_nomi_session(
        &self,
        conversation_id: &str,
        _conversation_created_at: i64,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<NomiSessionResetOutcome, AppError>>
                + Send,
        >,
    > {
        let conversation_id = conversation_id.to_owned();
        Box::pin(async move {
            Err(AppError::Internal(format!(
                "Agent runtime registry has no Nomi session persistence configured for conversation {conversation_id}"
            )))
        })
    }

    /// Terminate and remove every active runtime.
    fn terminate_all(&self);

    /// Number of fully initialized active runtimes.
    fn active_runtime_count(&self) -> usize;

    /// Whether this boot still owns any runtime lifecycle state for a
    /// conversation, including an uninitialized or teardown-quarantined slot.
    ///
    /// The default preserves compatibility for registries without separate
    /// lifecycle bookkeeping.
    fn has_registered_runtime(&self, conversation_id: &str) -> bool {
        self.get_runtime(conversation_id).is_some()
    }

    /// Collect runtimes eligible for idle cleanup.
    ///
    /// Returns conversation IDs of runtimes that:
    /// - have `status == Some(Finished)`
    /// - have been idle longer than `idle_threshold_ms`
    fn collect_idle_runtimes(&self, idle_threshold_ms: TimestampMs) -> Vec<String>;

    /// Revalidate and terminate one runtime previously reported as idle.
    ///
    /// Implementations must make the eligibility check and removal atomic with
    /// respect to runtime creation/replacement. The conservative default keeps
    /// custom/test registries source-compatible without allowing the generic
    /// scanner to terminate a runtime from a stale conversation-id snapshot.
    async fn terminate_idle_runtime_if_eligible(
        &self,
        conversation_id: &str,
        idle_threshold_ms: TimestampMs,
    ) -> Result<bool, AppError> {
        let _ = (conversation_id, idle_threshold_ms);
        Ok(false)
    }
}

/// Per-conversation slot: an [`OnceCell`] that the first concurrent caller
/// initialises by running the factory, and that every subsequent caller
/// awaits. Failed initialisations leave the cell empty so the next caller
/// may retry; the slot itself is only removed by explicit termination.
type RuntimeSlot = Arc<OnceCell<AgentRuntimeHandle>>;

#[derive(Clone)]
struct RuntimeTurnAdmission {
    generation: u64,
    slot: RuntimeSlot,
}

#[derive(Clone)]
struct RuntimeWorkspaceBinding {
    slot: RuntimeSlot,
    lease: nomifun_knowledge::WorkspaceBindingLease,
}

#[derive(Clone)]
struct RuntimeModelBinding {
    slot: RuntimeSlot,
    binding: RuntimeModelConfigBinding,
}

fn options_carry_knowledge_metadata(extra: &serde_json::Value) -> bool {
    let Some(extra) = extra.as_object() else {
        return false;
    };
    [
        "knowledge_mounts",
        "knowledge_binding_signature",
        "knowledge_mounts_signature",
        "knowledge_writeback",
        "knowledge_writeback_eagerness",
        "knowledge_channel_write_enabled",
    ]
    .iter()
    .any(|key| extra.contains_key(*key))
}

/// Max crash-evictions within [`RESTART_WINDOW_MS`] before a conversation's
/// respawn is refused. Beyond this the agent is deterministically crash-looping
/// and respawning again just burns a fresh CLI process + ACP handshake to die
/// the same way.
const RESTART_MAX_PER_WINDOW: u32 = 3;
/// Sliding window (ms) over which crash-evictions are counted. A conversation
/// that survives this long without a crash resets its budget, so a single crash
/// deep into a long session never trips the breaker.
const RESTART_WINDOW_MS: i64 = 60_000;
/// Base respawn backoff (ms); doubled per crash within the window (1s, 2s, …)
/// and capped at [`RESTART_MAX_BACKOFF_MS`]. Paces a flapping agent so a
/// zero-delay respawn cannot re-enter the same crashing operation instantly.
const RESTART_BASE_BACKOFF_MS: u64 = 500;
const RESTART_MAX_BACKOFF_MS: u64 = 8_000;

/// Cooldown after the SECOND consecutive factory-build failure (ms), doubled
/// per further failure and capped at [`BUILD_FAILURE_COOLDOWN_MAX_MS`].
/// The first failure imposes no cooldown — a user reading the error card and
/// clicking retry must not be told to wait — but automated senders (channels,
/// cron, AutoWork) that hammer a conversation whose agent cannot even finish
/// its startup handshake reach failure #2 immediately and get paced instead
/// of burning a fresh CLI spawn + full handshake timeout per message.
const BUILD_FAILURE_COOLDOWN_BASE_MS: i64 = 30_000;
const BUILD_FAILURE_COOLDOWN_MAX_MS: i64 = 120_000;
/// A build-failure streak is forgotten after this much time without another
/// failure, so yesterday's incident never delays today's first message.
const BUILD_FAILURE_RESET_MS: i64 = 600_000;

const MAX_CACHED_LIFECYCLE_GATES: usize = 256;
const BROKEN_RUNTIME_TEARDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(7);

#[derive(Clone, Copy)]
struct RestartRecord {
    /// Crash-evictions inside the current window.
    count: u32,
    /// When the current window began (`now_ms`).
    window_start_ms: i64,
}

/// Consecutive factory-build failures for a conversation. Unlike
/// [`RestartRecord`] (crash-evictions of a RUNNING agent, fixed window),
/// build failures are tracked as a streak keyed on the LAST failure: a
/// single handshake timeout already takes minutes, so a fixed 60s window
/// would reset between every two failures and the breaker would never
/// engage for exactly the failure mode it exists to pace.
#[derive(Clone, Copy)]
struct BuildFailureRecord {
    /// Consecutive failures without an intervening success.
    consecutive: u32,
    /// When the most recent failure was recorded (`now_ms`).
    last_failure_ms: i64,
}

/// Crash-loop governor for agent (re)builds.
///
/// A companion ACP agent that repeatedly crashes mid-turn (e.g. a native fault
/// in the Computer/a11y C-FFI, which no Rust error boundary can catch) is
/// evicted with [`AgentKillReason::AgentErrorRecovery`] and lazily respawned on
/// the next drive. With no throttle that respawn is instant, so a deterministic
/// crash re-faults within seconds — the "6-second restart loop" users observe.
///
/// This governor bounds that loop per conversation: it counts crash-evictions
/// within a sliding window, applies exponential backoff before each respawn,
/// and once [`RESTART_MAX_PER_WINDOW`] crashes occur inside [`RESTART_WINDOW_MS`]
/// it refuses to respawn (the caller surfaces a terminal error the UI renders as
/// "paused — crash looping") until the window elapses. Only genuine crashes are
/// counted; deliberate recycles (idle timeout, knowledge-binding rebuild,
/// conversation delete) never consume the budget, so a healthy reopen
/// is never throttled. Modelled on the MCP stdio transport's respawn breaker.
#[derive(Default)]
struct RestartGovernor {
    records: DashMap<String, RestartRecord>,
    /// Factory-build failure streaks (see [`BuildFailureRecord`]). Kept
    /// separate from crash records: a failed build never produced a process
    /// to crash, and its pacing policy (streak cooldown) differs from the
    /// crash policy (windowed budget + pre-spawn backoff).
    build_failures: DashMap<String, BuildFailureRecord>,
}

impl RestartGovernor {
    /// Record a crash-eviction for `conversation_id` at `now_ms`; returns the
    /// crash count within the (possibly just-reset) window.
    fn record_crash(&self, conversation_id: &str, now_ms: i64) -> u32 {
        let mut rec = self
            .records
            .entry(conversation_id.to_owned())
            .or_insert(RestartRecord {
                count: 0,
                window_start_ms: now_ms,
            });
        if now_ms - rec.window_start_ms > RESTART_WINDOW_MS {
            rec.count = 1;
            rec.window_start_ms = now_ms;
        } else {
            rec.count += 1;
        }
        rec.count
    }

    /// Decide whether a (re)build for `conversation_id` may proceed at `now_ms`.
    ///
    /// - `Ok(backoff_ms)` — proceed after sleeping `backoff_ms` (0 when there is
    ///   no recent crash history or the window has elapsed).
    /// - `Err(count)` — refuse: the conversation crashed `count` times within
    ///   the window and is crash-looping.
    fn gate(&self, conversation_id: &str, now_ms: i64) -> Result<u64, u32> {
        match self.records.get(conversation_id) {
            Some(rec) if now_ms - rec.window_start_ms <= RESTART_WINDOW_MS => {
                if rec.count >= RESTART_MAX_PER_WINDOW {
                    Err(rec.count)
                } else {
                    let backoff = RESTART_BASE_BACKOFF_MS
                        .saturating_mul(1u64 << rec.count.min(5))
                        .min(RESTART_MAX_BACKOFF_MS);
                    Ok(backoff)
                }
            }
            // No record, or the window has elapsed → unthrottled build.
            _ => Ok(0),
        }
    }

    /// Drop all crash bookkeeping for a conversation (definitive teardown).
    fn forget(&self, conversation_id: &str) {
        self.records.remove(conversation_id);
        self.build_failures.remove(conversation_id);
    }

    fn clear(&self) {
        self.records.clear();
        self.build_failures.clear();
    }

    /// Record a factory-build failure at `now_ms`; returns the streak length.
    /// A failure after [`BUILD_FAILURE_RESET_MS`] of quiet starts a new streak.
    fn record_build_failure(&self, conversation_id: &str, now_ms: i64) -> u32 {
        let mut rec = self
            .build_failures
            .entry(conversation_id.to_owned())
            .or_insert(BuildFailureRecord {
                consecutive: 0,
                last_failure_ms: now_ms,
            });
        if now_ms - rec.last_failure_ms > BUILD_FAILURE_RESET_MS {
            rec.consecutive = 1;
        } else {
            rec.consecutive += 1;
        }
        rec.last_failure_ms = now_ms;
        rec.consecutive
    }

    /// Forget the build-failure streak after a successful build.
    fn record_build_success(&self, conversation_id: &str) {
        self.build_failures.remove(conversation_id);
    }

    /// Decide whether a fresh factory build may start at `now_ms`.
    ///
    /// - `Ok(())` — no recent failure streak (or the streak's cooldown has
    ///   elapsed); build immediately.
    /// - `Err(remaining_ms)` — the conversation's last builds failed
    ///   back-to-back and the escalating cooldown has not elapsed yet.
    ///   Callers refuse fast instead of burning another spawn + handshake.
    fn build_gate(&self, conversation_id: &str, now_ms: i64) -> Result<(), i64> {
        let Some(rec) = self.build_failures.get(conversation_id) else {
            return Ok(());
        };
        if now_ms - rec.last_failure_ms > BUILD_FAILURE_RESET_MS {
            return Ok(());
        }
        // First failure: retry freely. From the second consecutive failure
        // onward: 30s, 60s, 120s (capped).
        if rec.consecutive < 2 {
            return Ok(());
        }
        let exponent = (rec.consecutive - 2).min(8);
        let cooldown = BUILD_FAILURE_COOLDOWN_BASE_MS
            .saturating_mul(1i64 << exponent)
            .min(BUILD_FAILURE_COOLDOWN_MAX_MS);
        let elapsed = now_ms - rec.last_failure_ms;
        if elapsed >= cooldown {
            Ok(())
        } else {
            Err(cooldown - elapsed)
        }
    }
}

/// Default implementation of [`AgentRuntimeRegistry`] using a concurrent hash map.
#[derive(Clone)]
pub struct InMemoryAgentRuntimeRegistry {
    runtimes: Arc<DashMap<String, RuntimeSlot>>,
    /// Slots whose awaitable teardown failed. They remain authoritative until
    /// the exact runtime's process exit is proven; a replacement must never be
    /// admitted merely because the public runtime map was temporarily removed
    /// for teardown.
    teardown_quarantine: Arc<DashMap<String, RuntimeSlot>>,
    /// Latest turn generation admitted to each exact runtime slot. A delayed
    /// cancellation may only quarantine the slot it originally admitted.
    turn_admissions: Arc<DashMap<String, RuntimeTurnAdmission>>,
    /// Physical workspace mount authority attached to the exact runtime slot.
    /// It is removed only after that slot's process exit is proven.  In
    /// particular, teardown quarantine retains this lease so another
    /// conversation cannot reconfigure `.nomi/knowledge` while the old
    /// process may still be alive.
    workspace_bindings: Arc<DashMap<String, RuntimeWorkspaceBinding>>,
    /// Exact provider graph attached to each live Nomi runtime slot. A slot is
    /// reusable only while a fresh resolver read returns the same binding.
    model_config_bindings: Arc<DashMap<String, RuntimeModelBinding>>,
    /// Serializes build and awaitable teardown for each conversation. The gate
    /// intentionally outlives a removed runtime slot so no replacement factory
    /// can start while the old agent is still unwinding.
    lifecycle_gates: Arc<DashMap<String, Weak<AsyncMutex<()>>>>,
    factory: AgentRuntimeFactory,
    model_config_resolver: Option<AgentRuntimeModelConfigResolver>,
    /// Optional only for source-compatible custom/test construction. Product
    /// composition configures this to `{data_dir}/nomi-sessions`; reset fails
    /// closed while it is absent.
    nomi_session_persistence: Option<Arc<NomiSessionPersistence>>,
    /// Bounds rapid crash-respawn loops per conversation (see [`RestartGovernor`]).
    governor: Arc<RestartGovernor>,
    /// The exact slot whose crash-eviction was last fed to the governor, per
    /// conversation. Crash accounting is per runtime *instance*: retried or
    /// overlapping teardown requests for the same slot (e.g.
    /// `terminate_runtime_until_confirmed`'s retry loop over a stuck
    /// quarantined runtime) must consume exactly one unit of restart budget.
    counted_crash_slots: Arc<DashMap<String, Weak<OnceCell<AgentRuntimeHandle>>>>,
}

impl InMemoryAgentRuntimeRegistry {
    pub fn new(factory: AgentRuntimeFactory) -> Self {
        Self {
            runtimes: Arc::new(DashMap::new()),
            teardown_quarantine: Arc::new(DashMap::new()),
            turn_admissions: Arc::new(DashMap::new()),
            workspace_bindings: Arc::new(DashMap::new()),
            model_config_bindings: Arc::new(DashMap::new()),
            lifecycle_gates: Arc::new(DashMap::new()),
            factory,
            model_config_resolver: None,
            nomi_session_persistence: None,
            governor: Arc::new(RestartGovernor::default()),
            counted_crash_slots: Arc::new(DashMap::new()),
        }
    }

    /// Bind long-lived Nomi runtime reuse to the exact ModelInvoke Chat
    /// capability revision. Desktop product composition must install this;
    /// registries hosting only external/non-provider-managed agents may omit
    /// it because no provider secret or endpoint is cached in those runtimes.
    pub fn with_model_config_resolver(
        mut self,
        resolver: AgentRuntimeModelConfigResolver,
    ) -> Self {
        self.model_config_resolver = Some(resolver);
        self
    }

    /// Configure the exact directory shared with Nomi's `SessionManager`.
    ///
    /// This builder keeps [`Self::new`] source-compatible for registries that
    /// never host persisted Nomi sessions while making product reset support an
    /// explicit composition decision.
    pub fn with_nomi_session_directory(mut self, session_directory: PathBuf) -> Self {
        self.nomi_session_persistence =
            Some(Arc::new(NomiSessionPersistence::new(session_directory)));
        self
    }

    /// Look up a fully initialized runtime by conversation ID.
    fn initialized_runtime(&self, conversation_id: &str) -> Option<AgentRuntimeHandle> {
        let slot = self.runtimes.get(conversation_id)?.value().clone();
        if self.slot_is_quarantined(conversation_id, &slot) {
            return None;
        }
        let runtime = slot
            .get()
            .cloned()
            .filter(AgentRuntimeHandle::is_transport_healthy)?;
        // `OnceCell` becomes visible immediately after the factory returns,
        // before the admission path performs its post-build revision check.
        // Do not expose a product Nomi runtime through `get_runtime` during
        // that window (or if its binding bookkeeping is ever lost).
        if self.model_config_resolver.is_some()
            && runtime.agent_type() == AgentType::Nomi
            && self
                .model_config_binding_for_slot(conversation_id, &slot)
                .is_none()
        {
            return None;
        }
        Some(runtime)
    }

    fn lifecycle_gate(&self, conversation_id: &str) -> Arc<AsyncMutex<()>> {
        if self.lifecycle_gates.len() >= MAX_CACHED_LIFECYCLE_GATES {
            self.lifecycle_gates.retain(|_, gate| gate.strong_count() > 0);
        }
        if let Some(gate) = self
            .lifecycle_gates
            .get(conversation_id)
            .and_then(|gate| gate.upgrade())
        {
            return gate;
        }

        let candidate = Arc::new(AsyncMutex::new(()));
        match self.lifecycle_gates.entry(conversation_id.to_owned()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                if let Some(gate) = entry.get().upgrade() {
                    return gate;
                }
                entry.insert(Arc::downgrade(&candidate));
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(Arc::downgrade(&candidate));
            }
        }
        candidate
    }

    fn quarantined_slot(&self, conversation_id: &str) -> Option<RuntimeSlot> {
        self.teardown_quarantine
            .get(conversation_id)
            .map(|entry| entry.value().clone())
    }

    fn slot_is_quarantined(&self, conversation_id: &str, slot: &RuntimeSlot) -> bool {
        self.teardown_quarantine
            .get(conversation_id)
            .is_some_and(|quarantined| Arc::ptr_eq(quarantined.value(), slot))
    }

    fn clear_quarantine_if_matches(&self, conversation_id: &str, slot: &RuntimeSlot) {
        self.teardown_quarantine
            .remove_if(conversation_id, |_, current| Arc::ptr_eq(current, slot));
    }

    /// Fail before runtime creation/reuse when another live turn generation is
    /// still authoritative. The lifecycle gate must be held by the caller.
    fn ensure_turn_generation_available(
        &self,
        conversation_id: &str,
        generation: u64,
    ) -> Result<(), AppError> {
        if generation == 0 {
            return Err(AppError::Conflict(format!(
                "Agent runtime turn generation for conversation {conversation_id} must be positive"
            )));
        }
        if let Some(active) = self.turn_admissions.get(conversation_id)
            && active.generation != generation
        {
            return Err(AppError::Conflict(format!(
                "conversation {conversation_id} already has an active runtime turn generation"
            )));
        }
        Ok(())
    }

    /// Bind a turn generation to one exact runtime slot without ever replacing
    /// a different active generation or slot. The lifecycle gate must be held
    /// by the caller.
    fn bind_turn_admission_exact(
        &self,
        conversation_id: &str,
        generation: u64,
        slot: &RuntimeSlot,
    ) -> Result<(), AppError> {
        self.ensure_turn_generation_available(conversation_id, generation)?;
        match self.turn_admissions.entry(conversation_id.to_owned()) {
            dashmap::mapref::entry::Entry::Occupied(entry)
                if entry.get().generation == generation
                    && Arc::ptr_eq(&entry.get().slot, slot) =>
            {
                Ok(())
            }
            dashmap::mapref::entry::Entry::Occupied(_) => Err(AppError::Conflict(format!(
                "conversation {conversation_id} runtime turn admission belongs to another exact slot"
            ))),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(RuntimeTurnAdmission {
                    generation,
                    slot: Arc::clone(slot),
                });
                Ok(())
            }
        }
    }

    fn clear_turn_admission_if_matches(&self, conversation_id: &str, slot: &RuntimeSlot) {
        self.turn_admissions.remove_if(conversation_id, |_, admission| {
            Arc::ptr_eq(&admission.slot, slot)
        });
    }

    fn clear_workspace_binding_if_matches(&self, conversation_id: &str, slot: &RuntimeSlot) {
        self.workspace_bindings
            .remove_if(conversation_id, |_, binding| Arc::ptr_eq(&binding.slot, slot));
    }

    fn clear_model_config_binding_if_matches(
        &self,
        conversation_id: &str,
        slot: &RuntimeSlot,
    ) {
        self.model_config_bindings
            .remove_if(conversation_id, |_, binding| Arc::ptr_eq(&binding.slot, slot));
    }

    fn model_config_binding_for_slot(
        &self,
        conversation_id: &str,
        slot: &RuntimeSlot,
    ) -> Option<RuntimeModelConfigBinding> {
        self.model_config_bindings
            .get(conversation_id)
            .filter(|binding| Arc::ptr_eq(&binding.slot, slot))
            .map(|binding| binding.binding.clone())
    }

    fn slot_has_active_turn(&self, conversation_id: &str, slot: &RuntimeSlot) -> bool {
        self.turn_admissions
            .get(conversation_id)
            .is_some_and(|admission| Arc::ptr_eq(&admission.slot, slot))
    }

    fn attach_model_config_binding(
        &self,
        conversation_id: &str,
        slot: &RuntimeSlot,
        binding: RuntimeModelConfigBinding,
    ) -> Result<(), AppError> {
        match self.model_config_bindings.entry(conversation_id.to_owned()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                if !Arc::ptr_eq(&entry.get().slot, slot) {
                    return Err(AppError::Conflict(format!(
                        "conversation {conversation_id} has model configuration bound to a different runtime generation"
                    )));
                }
                entry.insert(RuntimeModelBinding {
                    slot: Arc::clone(slot),
                    binding,
                });
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(RuntimeModelBinding {
                    slot: Arc::clone(slot),
                    binding,
                });
            }
        }
        Ok(())
    }

    async fn resolve_model_config_binding(
        &self,
        agent_type: AgentType,
        model: Option<&ProviderWithModel>,
    ) -> Result<Option<RuntimeModelConfigBinding>, AppError> {
        if agent_type != AgentType::Nomi {
            return Ok(None);
        }
        let Some(resolver) = self.model_config_resolver.as_ref() else {
            // Standalone and integration-test registries may host factories
            // that do not cache provider transport state. Product composition
            // always supplies the ModelInvoke-backed resolver.
            return Ok(None);
        };
        let model = model.cloned().ok_or_else(|| {
            AppError::BadRequest("Nomi runtime requires a provider and model".to_owned())
        })?;
        resolver(model).await.map(Some)
    }

    /// Attach a pre-acquired physical workspace binding to `slot`.
    ///
    /// An initialized runtime with no exact attachment has unknown authority:
    /// it may have been built before mount fencing existed or by a custom
    /// registry path.  Never "adopt" it after the fact, because the mount
    /// namespace may already have changed underneath that process.
    fn attach_workspace_binding(
        &self,
        conversation_id: &str,
        slot: &RuntimeSlot,
        requested: Option<&nomifun_knowledge::WorkspaceBindingLease>,
        initialized_runtime: bool,
    ) -> Result<(), AppError> {
        let Some(requested) = requested else {
            if let Some(current) = self.workspace_bindings.get(conversation_id) {
                let generation = if Arc::ptr_eq(&current.slot, slot) {
                    "the exact cached runtime"
                } else {
                    "a different runtime generation"
                };
                return Err(AppError::Conflict(format!(
                    "conversation {conversation_id} has knowledge workspace authority attached to {generation}; reuse requires the same exact binding lease"
                )));
            }
            return Ok(());
        };

        match self.workspace_bindings.entry(conversation_id.to_owned()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                let current = entry.get();
                if !Arc::ptr_eq(&current.slot, slot) {
                    return Err(AppError::Conflict(format!(
                        "conversation {conversation_id} has workspace authority attached to a different runtime generation"
                    )));
                }
                if !current.lease.same_binding(requested) {
                    return Err(AppError::Conflict(format!(
                        "conversation {conversation_id} cannot replace a live runtime's knowledge workspace binding"
                    )));
                }
                // Replace the caller's lease handle while it is still alive;
                // dropping the old handle therefore cannot open an authority
                // gap even if it happened to be the registry's last clone.
                entry.insert(RuntimeWorkspaceBinding {
                    slot: Arc::clone(slot),
                    lease: requested.clone(),
                });
                Ok(())
            }
            dashmap::mapref::entry::Entry::Vacant(entry) if !initialized_runtime => {
                entry.insert(RuntimeWorkspaceBinding {
                    slot: Arc::clone(slot),
                    lease: requested.clone(),
                });
                Ok(())
            }
            dashmap::mapref::entry::Entry::Vacant(_) => Err(AppError::Conflict(format!(
                "conversation {conversation_id} has a live runtime with unknown knowledge workspace authority"
            ))),
        }
    }

    fn quarantine_teardown_slot(&self, conversation_id: &str, slot: &RuntimeSlot) {
        self.teardown_quarantine
            .insert(conversation_id.to_owned(), Arc::clone(slot));
        match self.runtimes.entry(conversation_id.to_owned()) {
            dashmap::mapref::entry::Entry::Occupied(entry) => {
                if !Arc::ptr_eq(entry.get(), slot) {
                    warn!(
                        conversation_id,
                        "A different runtime occupied a failed teardown slot; preserving the failed runtime in quarantine"
                    );
                }
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(Arc::clone(slot));
            }
        }
    }

    /// Kill one exact runtime while its caller holds the lifecycle gate.
    ///
    /// The slot is quarantined before the first await and remains registered
    /// until process-tree exit is proven. This makes future cancellation safe:
    /// dropping the teardown future cannot create an empty admission window.
    async fn teardown_slot_under_gate(
        &self,
        conversation_id: &str,
        slot: RuntimeSlot,
        reason: Option<AgentKillReason>,
        timeout: Option<std::time::Duration>,
    ) -> Result<(), AppError> {
        self.quarantine_teardown_slot(conversation_id, &slot);
        let Some(agent) = slot.get().cloned() else {
            // The lifecycle gate proves no factory can still be filling this
            // OnceCell. An empty quarantined slot therefore owns no process.
            self.runtimes
                .remove_if(conversation_id, |_, current| Arc::ptr_eq(current, &slot));
            self.clear_quarantine_if_matches(conversation_id, &slot);
            self.clear_turn_admission_if_matches(conversation_id, &slot);
            self.clear_workspace_binding_if_matches(conversation_id, &slot);
            self.clear_model_config_binding_if_matches(conversation_id, &slot);
            return Ok(());
        };

        let result = match timeout {
            Some(timeout) => match tokio::time::timeout(timeout, agent.kill_and_wait(reason)).await {
                Ok(result) => result,
                Err(_) => Err(AppError::Timeout(format!(
                    "Agent runtime teardown timed out for conversation {conversation_id}"
                ))),
            },
            None => agent.kill_and_wait(reason).await,
        };

        match result {
            Ok(()) => {
                self.runtimes
                    .remove_if(conversation_id, |_, current| Arc::ptr_eq(current, &slot));
                self.clear_quarantine_if_matches(conversation_id, &slot);
                self.clear_turn_admission_if_matches(conversation_id, &slot);
                // Process-tree exit is the authority release point.  Failed
                // teardown keeps this exact slot quarantined and deliberately
                // retains its physical workspace binding below.
                self.clear_workspace_binding_if_matches(conversation_id, &slot);
                self.clear_model_config_binding_if_matches(conversation_id, &slot);
                Ok(())
            }
            Err(error) => {
                self.quarantine_teardown_slot(conversation_id, &slot);
                Err(error)
            }
        }
    }

    async fn terminate_registered_runtime(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> Result<(), AppError> {
        let lifecycle_gate = self.lifecycle_gate(conversation_id);
        let _lifecycle = lifecycle_gate.lock().await;
        let slot = self.quarantined_slot(conversation_id).or_else(|| {
            self.runtimes
                .get(conversation_id)
                .map(|entry| entry.value().clone())
        });
        self.note_termination_for_governor(conversation_id, reason, slot.as_ref());
        let Some(slot) = slot else {
            return Ok(());
        };

        info!(
            conversation_id,
            ?reason,
            "Terminating Agent runtime and awaiting proven shutdown"
        );
        self.teardown_slot_under_gate(conversation_id, slot, reason, None)
            .await
    }

    /// Feed a termination into the restart governor. Only a crash-recovery eviction
    /// ([`AgentKillReason::AgentErrorRecovery`]) counts against the respawn
    /// budget; every other termination is a deliberate recycle and must not. A
    /// definitive teardown ([`AgentKillReason::ConversationDeleted`]) drops the
    /// bookkeeping so a reused conversation id starts fresh.
    ///
    /// Crash accounting is per runtime *instance*: `slot` is the exact slot
    /// being torn down. A termination that resolves no slot tears down nothing
    /// and records nothing (an administrative teardown request against an idle
    /// conversation is not a crash), and repeated teardown requests for the
    /// same slot — retry loops awaiting proof, overlapping recovery owners —
    /// consume exactly one unit of budget.
    fn note_termination_for_governor(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
        slot: Option<&RuntimeSlot>,
    ) {
        match reason {
            Some(AgentKillReason::AgentErrorRecovery) => {
                let Some(slot) = slot else { return };
                let slot = Arc::downgrade(slot);
                let already_counted = self
                    .counted_crash_slots
                    .get(conversation_id)
                    .is_some_and(|counted| Weak::ptr_eq(counted.value(), &slot));
                if already_counted {
                    return;
                }
                self.counted_crash_slots
                    .insert(conversation_id.to_owned(), slot);
                let count = self.governor.record_crash(conversation_id, now_ms());
                warn!(
                    conversation_id,
                    crash_count = count,
                    "Recorded agent crash-eviction for the restart governor"
                );
            }
            Some(AgentKillReason::ConversationDeleted) => {
                self.counted_crash_slots.remove(conversation_id);
                self.governor.forget(conversation_id);
            }
            _ => {}
        }
    }

    async fn get_or_create_runtime_inner(
        &self,
        conversation_id: &str,
        turn_generation: Option<u64>,
        cancellation: Option<CancellationToken>,
        mut options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        if options.workspace_binding_lease.is_none() {
            return Err(AppError::Conflict(format!(
                "Agent runtime build for conversation {conversation_id} requires an exact physical workspace binding lease"
            )));
        }
        if let Some(binding) = options.workspace_binding_lease.as_ref()
            && !binding.matches_workspace(Path::new(&options.workspace))?
        {
            return Err(AppError::Conflict(format!(
                "Agent runtime build for conversation {conversation_id} carries a knowledge binding lease for a different physical workspace"
            )));
        }
        if let Some(binding) = options.workspace_binding_lease.as_ref()
            && binding.is_unbound()
            && options_carry_knowledge_metadata(&options.extra)
        {
            return Err(AppError::Conflict(format!(
                "Agent runtime build for conversation {conversation_id} cannot carry knowledge mount metadata under an unbound workspace lease"
            )));
        }

        // Keep the pre-acquired authority alive while waiting for lifecycle
        // admission, then transfer a clone to the exact slot before a factory
        // can spawn any process.
        let requested_workspace_binding = options.workspace_binding_lease.take();
        let lifecycle_gate = self.lifecycle_gate(conversation_id);
        let _lifecycle = if let Some(cancellation) = cancellation.as_ref() {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    return Err(AppError::Conflict(format!(
                        "Agent runtime build for conversation {conversation_id} was cancelled while waiting for lifecycle admission"
                    )));
                }
                guard = lifecycle_gate.lock() => guard,
            }
        } else {
            lifecycle_gate.lock().await
        };
        if cancellation.as_ref().is_some_and(CancellationToken::is_cancelled) {
            return Err(AppError::Conflict(format!(
                "Agent runtime build for conversation {conversation_id} was cancelled before initialization"
            )));
        }

        if let Some(quarantined) = self.quarantined_slot(conversation_id) {
            warn!(
                conversation_id,
                "Retrying teardown of quarantined Agent runtime before replacement admission"
            );
            if let Err(error) = self
                .teardown_slot_under_gate(
                    conversation_id,
                    quarantined,
                    Some(AgentKillReason::AgentErrorRecovery),
                    Some(BROKEN_RUNTIME_TEARDOWN_GRACE),
                )
                .await
            {
                warn!(
                    conversation_id,
                    error = %ErrorChain(&error),
                    "Quarantined Agent runtime teardown still failed; replacement remains blocked"
                );
                return Err(error);
            }
        }
        if let Some(turn_generation) = turn_generation {
            // A normal successor must wait for the prior durable/local owner
            // to release its exact admission. A cancelled/quarantined slot was
            // settled immediately above, which also cleared its binding.
            self.ensure_turn_generation_available(conversation_id, turn_generation)?;
        }

        let requested_agent_type = options.agent_type;
        let requested_model = options.model.clone();
        let requested_binding_result = self
            .resolve_model_config_binding(requested_agent_type, requested_model.as_ref())
            .await;
        let mut requested_model_binding = match requested_binding_result {
            Ok(binding) => binding,
            Err(error) => {
                // An invalid/disabled replacement graph is still a
                // configuration change. Once no turn owns the cached slot,
                // erase the runtime carrying the old key/endpoint before
                // surfacing the new configuration error.
                if let Some(slot) = self
                    .runtimes
                    .get(conversation_id)
                    .map(|entry| entry.value().clone())
                    && !self.slot_has_active_turn(conversation_id, &slot)
                {
                    self.teardown_slot_under_gate(
                        conversation_id,
                        slot,
                        Some(AgentKillReason::ConfigurationChanged),
                        None,
                    )
                    .await?;
                }
                return Err(error);
            }
        };

        let slot: RuntimeSlot = loop {
            let slot = self
                .runtimes
                .entry(conversation_id.to_owned())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone();
            let Some(runtime) = slot.get().cloned() else {
                break slot;
            };
            if self.slot_is_quarantined(conversation_id, &slot) {
                if let Err(error) = self
                    .teardown_slot_under_gate(
                        conversation_id,
                        slot,
                        Some(AgentKillReason::AgentErrorRecovery),
                        Some(BROKEN_RUNTIME_TEARDOWN_GRACE),
                    )
                    .await
                {
                    return Err(error);
                }
                continue;
            }
            if turn_generation.is_some() && runtime.requires_turn_boundary_recycle() {
                info!(
                    conversation_id,
                    "Recycling stateless Agent process at its completed turn boundary"
                );
                // This is a healthy protocol-boundary recycle, not crash
                // recovery. Keep the lifecycle gate and workspace authority
                // until exact process-tree exit is proven, then build the
                // successor in a fresh slot. No restart-governor record is
                // added for normal completed turns.
                self.teardown_slot_under_gate(
                    conversation_id,
                    slot,
                    Some(AgentKillReason::TurnBoundaryRecycle),
                    None,
                )
                .await?;
                continue;
            }
            if runtime.is_transport_healthy() {
                let cached_model_binding =
                    self.model_config_binding_for_slot(conversation_id, &slot);
                if self.model_config_resolver.is_some()
                    && cached_model_binding != requested_model_binding
                {
                    // A runtime that is still serving an admitted turn owns
                    // that exact provider snapshot until the turn reaches its
                    // terminal boundary. Never kill it from a concurrent
                    // preparation/send; fail closed and let the next attempt
                    // recycle after release.
                    if self.slot_has_active_turn(conversation_id, &slot) {
                        return Err(AppError::Conflict(format!(
                            "Agent runtime provider configuration changed during an active turn for conversation {conversation_id}; retry after the turn reaches a terminal boundary"
                        )));
                    }
                    info!(
                        conversation_id,
                        "Recycling Agent runtime because its provider configuration revision changed"
                    );
                    self.teardown_slot_under_gate(
                        conversation_id,
                        slot,
                        Some(AgentKillReason::ConfigurationChanged),
                        None,
                    )
                    .await?;
                    // Teardown can take long enough for another provider edit.
                    // Refresh before any replacement factory is admitted.
                    requested_model_binding = self
                        .resolve_model_config_binding(
                            requested_agent_type,
                            requested_model.as_ref(),
                        )
                        .await?;
                    continue;
                }
                self.attach_workspace_binding(
                    conversation_id,
                    &slot,
                    requested_workspace_binding.as_ref(),
                    true,
                )?;
                if let Some(turn_generation) = turn_generation {
                    // Reserve this Finished runtime against idle cleanup before
                    // releasing the lifecycle gate. `send_message` will move
                    // it to Running, but that happens after this call returns.
                    self.bind_turn_admission_exact(
                        conversation_id,
                        turn_generation,
                        &slot,
                    )?;
                    runtime.touch_activity();
                }
                return Ok(runtime);
            }

            // A permanent manager relay has exited or lost events. This slot
            // can never safely produce another terminal boundary, so evict and
            // await bounded teardown under the same lifecycle gate before a
            // replacement factory is admitted.
            self.note_termination_for_governor(
                conversation_id,
                Some(AgentKillReason::AgentErrorRecovery),
                Some(&slot),
            );
            warn!(conversation_id, "Evicting cached runtime with a broken event transport");
            if let Err(error) = self
                .teardown_slot_under_gate(
                    conversation_id,
                    slot,
                    Some(AgentKillReason::AgentErrorRecovery),
                    Some(BROKEN_RUNTIME_TEARDOWN_GRACE),
                )
                .await
            {
                warn!(
                    conversation_id,
                    error = %ErrorChain(&error),
                    "Broken Agent runtime teardown failed; replacement remains blocked"
                );
                return Err(error);
            }
        };

        self.attach_workspace_binding(
            conversation_id,
            &slot,
            requested_workspace_binding.as_ref(),
            false,
        )?;

        match self.governor.gate(conversation_id, now_ms()) {
            Ok(0) => {}
            Ok(backoff_ms) => {
                warn!(
                    conversation_id,
                    backoff_ms, "Backing off before respawning a recently-crashed agent"
                );
                if let Some(cancellation) = cancellation.as_ref() {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => {
                            self.runtimes.remove_if(conversation_id, |_, current| {
                                Arc::ptr_eq(current, &slot)
                            });
                            self.clear_workspace_binding_if_matches(conversation_id, &slot);
                            return Err(AppError::Conflict(format!(
                                "Agent runtime build for conversation {conversation_id} was cancelled during restart backoff"
                            )));
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
                    }
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
            Err(count) => {
                self.runtimes
                    .remove_if(conversation_id, |_, current| Arc::ptr_eq(current, &slot));
                self.clear_workspace_binding_if_matches(conversation_id, &slot);
                warn!(
                    conversation_id,
                    crash_count = count,
                    window_ms = RESTART_WINDOW_MS,
                    "Agent is crash-looping; refusing to respawn until the window elapses"
                );
                return Err(AppError::Conflict(format!(
                    "Agent for conversation {conversation_id} crashed {count} times within {}s and \
                     is paused to break a crash loop. Resolve the underlying failure (see the \
                     agent's exit code/signal in the logs), then try again shortly.",
                    RESTART_WINDOW_MS / 1000
                )));
            }
        }

        if cancellation.as_ref().is_some_and(CancellationToken::is_cancelled) {
            self.runtimes.remove_if(conversation_id, |_, current| Arc::ptr_eq(current, &slot));
            self.clear_workspace_binding_if_matches(conversation_id, &slot);
            return Err(AppError::Conflict(format!(
                "Agent runtime build for conversation {conversation_id} was cancelled before factory start"
            )));
        }

        let factory = self.factory.clone();
        // Build-failure streak accounting lives INSIDE the init closure so it
        // is exact under single-flight: when a failed init lets the next
        // queued waiter run its own attempt, each real factory run is counted
        // once, refusals during the cooldown are counted never, and the gate
        // check itself races nothing (the closure runs serially per slot).
        let governor = Arc::clone(&self.governor);
        let gated_conversation_id = conversation_id.to_owned();
        let build = slot.get_or_try_init(|| async move {
            // When the last builds failed back-to-back (e.g. the agent CLI
            // cannot finish its startup handshake), refuse fast during the
            // escalating cooldown instead of burning another CLI spawn +
            // full handshake timeout per message.
            if let Err(remaining_ms) = governor.build_gate(&gated_conversation_id, now_ms()) {
                let retry_in_secs = ((remaining_ms + 999) / 1000).max(1);
                warn!(
                    conversation_id = %gated_conversation_id,
                    remaining_ms, "Refusing agent build during build-failure cooldown"
                );
                return Err(AppError::Conflict(format!(
                    "The agent for conversation {gated_conversation_id} failed to start repeatedly \
                     and is cooling down. Try again in {retry_in_secs}s (see the previous startup \
                     error for the underlying cause)."
                )));
            }
            match factory(options).await {
                Ok(runtime) => {
                    governor.record_build_success(&gated_conversation_id);
                    Ok(runtime)
                }
                Err(error) => {
                    let streak =
                        governor.record_build_failure(&gated_conversation_id, now_ms());
                    warn!(
                        conversation_id = %gated_conversation_id,
                        consecutive_build_failures = streak,
                        error = %ErrorChain(&error),
                        "Agent runtime build failed"
                    );
                    Err(error)
                }
            }
        });
        tokio::pin!(build);
        let build_result = if let Some(cancellation) = cancellation.as_ref() {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    // Once a factory starts, dropping its future cannot prove
                    // that no subprocess escaped.  Drive it to a result while
                    // retaining workspace authority; if it produced a runtime,
                    // prove exact process-tree exit before releasing the slot.
                    match build.await {
                        Ok(_) => {
                            self.teardown_slot_under_gate(
                                conversation_id,
                                Arc::clone(&slot),
                                Some(AgentKillReason::UserCancelled),
                                None,
                            )
                            .await?;
                        }
                        Err(_) => {
                            self.runtimes.remove_if(conversation_id, |_, current| {
                                Arc::ptr_eq(current, &slot)
                            });
                            self.clear_workspace_binding_if_matches(conversation_id, &slot);
                        }
                    }
                    return Err(AppError::Conflict(format!(
                        "Agent runtime build for conversation {conversation_id} was cancelled during initialization"
                    )));
                }
                result = &mut build => result,
            }
        } else {
            build.await
        };
        let runtime = match build_result {
            Ok(runtime) => runtime.clone(),
            Err(error) => {
                self.runtimes
                    .remove_if(conversation_id, |_, current| Arc::ptr_eq(current, &slot));
                self.clear_workspace_binding_if_matches(conversation_id, &slot);
                return Err(error);
            }
        };

        let slot_is_current = self
            .runtimes
            .get(conversation_id)
            .is_some_and(|entry| Arc::ptr_eq(entry.value(), &slot));
        let cancelled = cancellation.as_ref().is_some_and(CancellationToken::is_cancelled);
        let teardown_requested = self.slot_is_quarantined(conversation_id, &slot);
        if cancelled || !slot_is_current || teardown_requested {
            let reason = cancelled.then_some(AgentKillReason::UserCancelled);
            self.teardown_slot_under_gate(conversation_id, Arc::clone(&slot), reason, None)
                .await?;
            return Err(AppError::Conflict(format!(
                "Agent runtime for conversation {conversation_id} was terminated while initializing"
            )));
        }

        // The factory resolves provider configuration independently. A save
        // that commits while it is building must therefore invalidate this
        // brand-new runtime before it can admit a turn; otherwise the process
        // could cache a stale key or endpoint despite the reuse check above.
        let confirmed_model_binding = match self
            .resolve_model_config_binding(requested_agent_type, requested_model.as_ref())
            .await
        {
            Ok(binding) => binding,
            Err(error) => {
                self.teardown_slot_under_gate(
                    conversation_id,
                    Arc::clone(&slot),
                    Some(AgentKillReason::ConfigurationChanged),
                    None,
                )
                .await?;
                return Err(error);
            }
        };
        if confirmed_model_binding != requested_model_binding {
            self.teardown_slot_under_gate(
                conversation_id,
                Arc::clone(&slot),
                Some(AgentKillReason::ConfigurationChanged),
                None,
            )
            .await?;
            return Err(AppError::Conflict(format!(
                "Agent runtime provider configuration changed while initializing for conversation {conversation_id}; retry to build from the current configuration"
            )));
        }
        if let Some(binding) = confirmed_model_binding {
            if let Err(error) =
                self.attach_model_config_binding(conversation_id, &slot, binding)
            {
                self.teardown_slot_under_gate(
                    conversation_id,
                    Arc::clone(&slot),
                    Some(AgentKillReason::ConfigurationChanged),
                    None,
                )
                .await?;
                return Err(error);
            }
        }
        if let Some(turn_generation) = turn_generation {
            if let Err(error) =
                self.bind_turn_admission_exact(conversation_id, turn_generation, &slot)
            {
                // This should be unreachable while the lifecycle gate is held,
                // but corrupted stale state must not leave a freshly built,
                // unowned process behind.
                self.teardown_slot_under_gate(
                    conversation_id,
                    Arc::clone(&slot),
                    Some(AgentKillReason::AgentErrorRecovery),
                    None,
                )
                .await?;
                return Err(error);
            }
            runtime.touch_activity();
        }
        Ok(runtime)
    }
}

#[async_trait]
impl AgentRuntimeRegistry for InMemoryAgentRuntimeRegistry {
    fn get_runtime(&self, conversation_id: &str) -> Option<AgentRuntimeHandle> {
        self.initialized_runtime(conversation_id)
    }

    async fn get_or_create_runtime(
        &self,
        conversation_id: &str,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        self.get_or_create_runtime_inner(conversation_id, None, None, options)
            .await
    }

    async fn get_or_create_runtime_for_preparation(
        &self,
        conversation_id: &str,
        cancellation: CancellationToken,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        self.get_or_create_runtime_inner(conversation_id, None, Some(cancellation), options)
            .await
    }

    async fn get_or_create_runtime_for_turn(
        &self,
        conversation_id: &str,
        turn_generation: u64,
        cancellation: CancellationToken,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        self.get_or_create_runtime_inner(
            conversation_id,
            Some(turn_generation),
            Some(cancellation),
            options,
        )
        .await
    }

    async fn release_runtime_turn(
        &self,
        conversation_id: &str,
        turn_generation: u64,
    ) -> Result<(), AppError> {
        if turn_generation == 0 {
            return Err(AppError::Conflict(format!(
                "Agent runtime turn generation for conversation {conversation_id} must be positive"
            )));
        }
        let lifecycle_gate = self.lifecycle_gate(conversation_id);
        let _lifecycle = lifecycle_gate.lock().await;
        let Some(active) = self
            .turn_admissions
            .get(conversation_id)
            .map(|entry| entry.value().clone())
        else {
            return Ok(());
        };
        if active.generation != turn_generation {
            // A delayed finalizer for an older turn cannot release a successor.
            return Ok(());
        }
        self.turn_admissions
            .remove_if(conversation_id, |_, current| {
                current.generation == turn_generation
                    && Arc::ptr_eq(&current.slot, &active.slot)
            });
        Ok(())
    }

    fn cancel_runtime_turn(
        &self,
        conversation_id: &str,
        turn_generation: u64,
        reason: Option<AgentKillReason>,
    ) -> Result<(), AppError> {
        let lifecycle_gate = self.lifecycle_gate(conversation_id);
        let _lifecycle = lifecycle_gate.try_lock().map_err(|_| {
            AppError::Conflict(format!(
                "Agent runtime lifecycle for conversation {conversation_id} is busy; refusing an unfenced turn cancellation"
            ))
        })?;
        let Some(admission) = self
            .turn_admissions
            .get(conversation_id)
            .map(|entry| entry.value().clone())
        else {
            return Ok(());
        };
        if admission.generation != turn_generation {
            return Ok(());
        }

        let exact_slot_is_current = self
            .runtimes
            .get(conversation_id)
            .is_some_and(|current| Arc::ptr_eq(current.value(), &admission.slot))
            || self.slot_is_quarantined(conversation_id, &admission.slot);
        if !exact_slot_is_current {
            self.clear_turn_admission_if_matches(conversation_id, &admission.slot);
            return Ok(());
        }

        self.note_termination_for_governor(conversation_id, reason, Some(&admission.slot));
        self.quarantine_teardown_slot(conversation_id, &admission.slot);
        if let Some(agent) = admission.slot.get() {
            agent.kill(reason)?;
        }
        Ok(())
    }

    fn terminate(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        let slot = self.quarantined_slot(conversation_id).or_else(|| {
            self.runtimes
                .get(conversation_id)
                .map(|entry| entry.value().clone())
        });
        self.note_termination_for_governor(conversation_id, reason, slot.as_ref());
        if let Some(slot) = slot {
            info!(conversation_id, ?reason, "Quarantining Agent runtime for termination");
            self.quarantine_teardown_slot(conversation_id, &slot);
            if let Some(agent) = slot.get() {
                agent.kill(reason)?;
            }
        }
        Ok(())
    }

    fn terminate_and_wait_result(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        // Governor accounting happens inside `terminate_registered_runtime`,
        // after the exact slot is resolved under the lifecycle gate — a
        // request that tears down nothing must not be booked as a crash.
        let registry = self.clone();
        let conversation_id = conversation_id.to_owned();
        Box::pin(async move {
            registry
                .terminate_registered_runtime(&conversation_id, reason)
                .await
        })
    }

    fn reset_persisted_nomi_session(
        &self,
        conversation_id: &str,
        conversation_created_at: i64,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<NomiSessionResetOutcome, AppError>>
                + Send,
        >,
    > {
        let registry = self.clone();
        let conversation_id = conversation_id.to_owned();
        Box::pin(async move {
            let persistence = registry
                .nomi_session_persistence
                .clone()
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "Nomi session persistence is not configured for conversation {conversation_id}"
                    ))
                })?;

            // Share the exact admission barrier used by factory construction
            // and awaitable teardown. If any runtime reappeared after teardown,
            // refuse to mutate a session it may be reading or saving.
            let lifecycle_gate = registry.lifecycle_gate(&conversation_id);
            let _lifecycle = lifecycle_gate.lock().await;
            if registry.has_registered_runtime(&conversation_id) {
                return Err(AppError::Conflict(format!(
                    "Agent runtime for conversation {conversation_id} is still registered; refusing persisted Nomi session reset"
                )));
            }

            let reset_conversation_id = conversation_id.clone();
            tokio::task::spawn_blocking(move || {
                persistence.reset_owned_session(
                    &reset_conversation_id,
                    conversation_created_at,
                )
            })
            .await
            .map_err(|error| {
                AppError::Internal(format!(
                    "Nomi session reset worker failed for conversation {conversation_id}: {error}"
                ))
            })?
        })
    }

    fn terminate_all(&self) {
        self.governor.clear();
        self.counted_crash_slots.clear();
        let keys: Vec<String> = self.runtimes.iter().map(|r| r.key().clone()).collect();
        for key in keys {
            if let Some(slot) = self
                .runtimes
                .get(&key)
                .map(|entry| entry.value().clone())
            {
                info!(conversation_id = %key, "Quarantining Agent runtime during registry shutdown");
                self.quarantine_teardown_slot(&key, &slot);
                if let Some(agent) = slot.get() {
                    let _ = agent.kill(None);
                }
            }
        }
    }

    fn active_runtime_count(&self) -> usize {
        self.runtimes
            .iter()
            .filter(|entry| {
                !self.slot_is_quarantined(entry.key(), entry.value())
                    && entry
                        .value()
                        .get()
                        .is_some_and(AgentRuntimeHandle::is_transport_healthy)
            })
            .count()
    }

    fn has_registered_runtime(&self, conversation_id: &str) -> bool {
        self.runtimes.contains_key(conversation_id)
            || self.teardown_quarantine.contains_key(conversation_id)
            || self.workspace_bindings.contains_key(conversation_id)
            || self.model_config_bindings.contains_key(conversation_id)
    }

    fn collect_idle_runtimes(&self, idle_threshold_ms: TimestampMs) -> Vec<String> {
        let now = now_ms();
        self.runtimes
            .iter()
            .filter_map(|entry| {
                if self.slot_is_quarantined(entry.key(), entry.value()) {
                    return None;
                }
                // Finished is an agent-local status, not proof that the
                // Conversation turn, continuation, receipt, or writeback has
                // reached its terminal boundary. Exact release is the only
                // authority that makes an admitted slot idle-collectable.
                if self.turn_admissions.contains_key(entry.key()) {
                    return None;
                }
                let agent = entry.value().get()?;
                if !agent.is_transport_healthy() {
                    return None;
                }
                // Only ACP agents participate in idle cleanup per API Spec
                (agent.agent_type() == AgentType::Acp
                    && agent.status() == Some(ConversationStatus::Finished)
                    && (now - agent.last_activity_at()) > idle_threshold_ms)
                    .then(|| entry.key().clone())
            })
            .collect()
    }

    async fn terminate_idle_runtime_if_eligible(
        &self,
        conversation_id: &str,
        idle_threshold_ms: TimestampMs,
    ) -> Result<bool, AppError> {
        let conversation_id = conversation_id.to_owned();
        let lifecycle_gate = self.lifecycle_gate(&conversation_id);
        let _lifecycle = lifecycle_gate.lock().await;

        let Some(slot) = self.runtimes.get(&conversation_id).map(|entry| entry.value().clone()) else {
            return Ok(false);
        };
        if self.slot_is_quarantined(&conversation_id, &slot) {
            return Ok(false);
        }
        if self.turn_admissions.contains_key(&conversation_id) {
            // Revalidate under the same lifecycle gate used by bind/release.
            // An agent may report Finished before durable post-processing is
            // closed; never let idle teardown erase exact cancel authority.
            return Ok(false);
        }
        let Some(agent) = slot.get().cloned() else {
            return Ok(false);
        };
        let now = now_ms();
        let still_idle = agent.is_transport_healthy()
            && agent.agent_type() == AgentType::Acp
            && agent.status() == Some(ConversationStatus::Finished)
            && (now - agent.last_activity_at()) > idle_threshold_ms;
        if !still_idle {
            return Ok(false);
        }

        info!(
            conversation_id,
            "Terminating revalidated idle Agent runtime (awaitable)"
        );
        self.teardown_slot_under_gate(
            &conversation_id,
            slot,
            Some(AgentKillReason::IdleTimeout),
            None,
        )
        .await?;
        Ok(true)
    }
}

/// Wired up by `nomifun-app` so deleting a conversation tears down its
/// agent process. Without this hook, ACP/nomi/nanobot subprocesses keep
/// streaming events for a `conversation_id` whose DB row is already gone
/// (Sentry ELECTRON-1BD).
#[async_trait]
impl OnConversationDelete for InMemoryAgentRuntimeRegistry {
    async fn on_conversation_deleted(&self, _user_id: &str, conversation_id: &str) {
        if let Err(e) = self.terminate(conversation_id, Some(AgentKillReason::ConversationDeleted)) {
            warn!(
                conversation_id,
                error = %ErrorChain(&e),
                "Failed to terminate Agent runtime on conversation delete (non-fatal)",
            );
        }
    }
}

/// Conversation-delete hook that removes a conversation's on-disk nomi state:
/// the global `nomi-sessions/*_{id}.json` file (+ index entry) and any
/// ID-named temp workspace under `work_dir/conversations`.
///
/// Without this, derived files outlive their authoritative entity. Cleanup
/// complements the per-session owner token and prevents stale state from being
/// observed through an incorrectly retained cache or filesystem reference.
/// Best-effort: every failure is logged, never fatal.
pub struct NomiSessionFilesCascade {
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
}

#[async_trait]
impl OnConversationDelete for NomiSessionFilesCascade {
    async fn on_conversation_deleted(&self, _user_id: &str, conversation_id: &str) {
        let id = conversation_id.to_string();

        // 1) nomi session transcript file + index entry.
        let session_manager = SessionManager::new(self.data_dir.join("nomi-sessions"), 100);
        if let Err(e) = session_manager.delete_session(&id) {
            warn!(conversation_id, error = %e, "Failed to delete nomi session file on conversation delete (non-fatal)");
        }

        // Managed UUIDv7 workspace deletion is owned by ConversationService
        // while it still has the authoritative temp_workspace_id.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_handle::{AgentRuntimeControl, MockAgentRuntime};
    use crate::protocol::events::AgentStreamEvent;
    use crate::types::SendMessageData;
    use futures_util::FutureExt;
    use nomi_types::message::{ContentBlock, Message, Role};
    use nomifun_common::{AgentKillReason, AgentType, ConversationStatus};
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use tokio::sync::{Semaphore, broadcast};

    /// A minimal mock Agent for testing runtime-registry logic. Lives behind
    /// the `AgentRuntimeHandle::Mock` trait-object variant so we don't have to
    /// stand up a real `AcpAgentManager` just to exercise lifecycle
    /// dispatch.
    struct MockAgent {
        agent_type: AgentType,
        conversation_id: String,
        workspace: String,
        status: Option<ConversationStatus>,
        last_activity: AtomicI64,
        event_tx: broadcast::Sender<AgentStreamEvent>,
        transport_healthy: Arc<AtomicBool>,
        turn_boundary_recycle_required: Arc<AtomicBool>,
        kill_started: Option<Arc<Semaphore>>,
        kill_release: Option<Arc<Semaphore>>,
        kill_error: Option<String>,
        kill_reasons: Option<Arc<std::sync::Mutex<Vec<Option<AgentKillReason>>>>>,
    }

    impl MockAgent {
        fn new(conversation_id: &str, status: Option<ConversationStatus>) -> Self {
            let (event_tx, _) = broadcast::channel(16);
            Self {
                agent_type: AgentType::Acp,
                conversation_id: conversation_id.to_owned(),
                workspace: "/tmp/test".to_owned(),
                status,
                last_activity: AtomicI64::new(now_ms()),
                event_tx,
                transport_healthy: Arc::new(AtomicBool::new(true)),
                turn_boundary_recycle_required: Arc::new(AtomicBool::new(false)),
                kill_started: None,
                kill_release: None,
                kill_error: None,
                kill_reasons: None,
            }
        }

        fn with_blocking_kill(mut self, started: Arc<Semaphore>, release: Arc<Semaphore>) -> Self {
            self.kill_started = Some(started);
            self.kill_release = Some(release);
            self
        }

        fn with_kill_error(mut self, error: impl Into<String>) -> Self {
            self.kill_error = Some(error.into());
            self
        }

        fn with_agent_type(mut self, t: AgentType) -> Self {
            self.agent_type = t;
            self
        }

        fn with_last_activity(mut self, ts: TimestampMs) -> Self {
            self.last_activity = AtomicI64::new(ts);
            self
        }

        fn with_transport_health(mut self, health: Arc<AtomicBool>) -> Self {
            self.transport_healthy = health;
            self
        }

        fn with_turn_boundary_recycle(mut self, required: Arc<AtomicBool>) -> Self {
            self.turn_boundary_recycle_required = required;
            self
        }

        fn with_kill_reasons(
            mut self,
            reasons: Arc<std::sync::Mutex<Vec<Option<AgentKillReason>>>>,
        ) -> Self {
            self.kill_reasons = Some(reasons);
            self
        }
    }

    #[async_trait::async_trait]
    impl AgentRuntimeControl for MockAgent {
        fn agent_type(&self) -> AgentType {
            self.agent_type
        }
        fn conversation_id(&self) -> &str {
            &self.conversation_id
        }
        fn workspace(&self) -> &str {
            &self.workspace
        }
        fn status(&self) -> Option<ConversationStatus> {
            self.status
        }
        fn is_transport_healthy(&self) -> bool {
            self.transport_healthy.load(Ordering::Acquire)
        }
        fn last_activity_at(&self) -> TimestampMs {
            self.last_activity.load(Ordering::Relaxed)
        }
        fn touch_activity(&self) {
            self.last_activity.store(now_ms(), Ordering::Relaxed);
        }
        fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
            self.event_tx.subscribe()
        }
        async fn send_message(
            &self,
            _data: SendMessageData,
        ) -> Result<(), crate::protocol::send_error::AgentSendError> {
            Ok(())
        }
        async fn cancel(&self) -> Result<(), AppError> {
            Ok(())
        }
        fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> {
            if let Some(reasons) = self.kill_reasons.as_ref() {
                reasons.lock().unwrap().push(reason);
            }
            Ok(())
        }
    }

    impl MockAgentRuntime for MockAgent {
        fn requires_turn_boundary_recycle(&self) -> bool {
            self.turn_boundary_recycle_required.load(Ordering::Acquire)
        }

        fn kill_and_wait(
            &self,
            reason: Option<AgentKillReason>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
            let kill_result = self.kill(reason);
            let started = self.kill_started.clone();
            let release = self.kill_release.clone();
            let kill_error = self.kill_error.clone();
            Box::pin(async move {
                kill_result?;
                if let Some(started) = started {
                    started.add_permits(1);
                }
                if let Some(release) = release {
                    let _ = release.acquire().await;
                }
                if let Some(error) = kill_error {
                    return Err(AppError::Internal(error));
                }
                Ok(())
            })
        }
    }

    fn runtime_test_workspace() -> &'static Path {
        static WORKSPACE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        WORKSPACE
            .get_or_init(|| tempfile::tempdir().expect("runtime registry test workspace"))
            .path()
    }

    fn make_runtime_options(conversation_id: &str) -> AgentRuntimeBuildOptions {
        let workspace = runtime_test_workspace();
        AgentRuntimeBuildOptions {
            user_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            agent_type: AgentType::Acp,
            workspace: workspace.to_string_lossy().into_owned(),
            model: None,
            conversation_id: conversation_id.into(),
            delegation_policy: Default::default(),
            extra: serde_json::Value::Null,
            conversation_created_at: None,
            workspace_binding_lease: Some(
                nomifun_knowledge::WorkspaceBindingLease::acquire_unbound(
                    workspace,
                    conversation_id,
                )
                .expect("runtime registry test unbound workspace lease"),
            ),
        }
    }

    fn mock_runtime(agent: MockAgent) -> AgentRuntimeHandle {
        AgentRuntimeHandle::Mock(Arc::new(agent))
    }

    fn make_registry() -> InMemoryAgentRuntimeRegistry {
        let factory: AgentRuntimeFactory = Arc::new(|opts: AgentRuntimeBuildOptions| {
            async move { Ok(mock_runtime(MockAgent::new(&opts.conversation_id, None))) }.boxed()
        });
        InMemoryAgentRuntimeRegistry::new(factory)
    }

    /// Two [`AgentRuntimeHandle`]s point to the same underlying agent iff they
    /// share an `Arc` — check by pointer identity on the inner trait object.
    fn same_mock(a: &AgentRuntimeHandle, b: &AgentRuntimeHandle) -> bool {
        match (a, b) {
            (AgentRuntimeHandle::Mock(x), AgentRuntimeHandle::Mock(y)) => Arc::ptr_eq(x, y),
            _ => false,
        }
    }

    #[test]
    fn get_runtime_returns_none_when_empty() {
        let registry = make_registry();
        assert!(registry.get_runtime("nonexistent").is_none());
    }

    #[tokio::test]
    async fn provider_config_revision_change_recycles_nomi_runtime_before_next_turn() {
        const CONVERSATION_ID: &str = "conv-provider-revision";
        const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000042";

        let revision = Arc::new(AtomicI64::new(1));
        let resolver_revision = Arc::clone(&revision);
        let resolver: AgentRuntimeModelConfigResolver = Arc::new(move |selection| {
            let revision = resolver_revision.load(Ordering::SeqCst);
            async move {
                Ok(RuntimeModelConfigBinding {
                    provider_id: selection.provider_id,
                    model: selection.use_model.unwrap_or(selection.model),
                    config_revision: revision,
                })
            }
            .boxed()
        });

        let factory_calls = Arc::new(AtomicUsize::new(0));
        let kill_reasons = Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls = Arc::clone(&factory_calls);
        let factory_kill_reasons = Arc::clone(&kill_reasons);
        let factory: AgentRuntimeFactory = Arc::new(move |options| {
            calls.fetch_add(1, Ordering::SeqCst);
            let kill_reasons = Arc::clone(&factory_kill_reasons);
            async move {
                Ok(mock_runtime(
                    MockAgent::new(&options.conversation_id, None)
                        .with_agent_type(AgentType::Nomi)
                        .with_kill_reasons(kill_reasons),
                ))
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory)
            .with_model_config_resolver(resolver);

        let options = || {
            let mut options = make_runtime_options(CONVERSATION_ID);
            options.agent_type = AgentType::Nomi;
            options.model = Some(ProviderWithModel {
                provider_id: PROVIDER_ID.to_owned(),
                model: "chat-model".to_owned(),
                use_model: None,
            });
            options
        };

        let first = registry
            .get_or_create_runtime_for_turn(
                CONVERSATION_ID,
                1,
                CancellationToken::new(),
                options(),
            )
            .await
            .unwrap();
        revision.store(2, Ordering::SeqCst);

        let active_conflict = registry
            .get_or_create_runtime_for_preparation(
                CONVERSATION_ID,
                CancellationToken::new(),
                options(),
            )
            .await;
        assert!(matches!(
            active_conflict,
            Err(AppError::Conflict(message)) if message.contains("active turn")
        ));
        assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
        assert!(kill_reasons.lock().unwrap().is_empty());

        registry.release_runtime_turn(CONVERSATION_ID, 1).await.unwrap();
        let second = registry
            .get_or_create_runtime_for_turn(
                CONVERSATION_ID,
                2,
                CancellationToken::new(),
                options(),
            )
            .await
            .unwrap();

        assert!(!same_mock(&first, &second));
        assert_eq!(factory_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            kill_reasons.lock().unwrap().as_slice(),
            &[Some(AgentKillReason::ConfigurationChanged)]
        );
        assert_eq!(
            registry
                .model_config_bindings
                .get(CONVERSATION_ID)
                .unwrap()
                .binding
                .config_revision,
            2
        );
    }

    #[tokio::test]
    async fn knowledge_runtime_metadata_without_workspace_lease_is_rejected_before_factory() {
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&factory_calls);
        let factory: AgentRuntimeFactory = Arc::new(move |options| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(mock_runtime(MockAgent::new(
                    &options.conversation_id,
                    None,
                )))
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let mut options = make_runtime_options("conv-unleased-knowledge");
        options.workspace_binding_lease = None;
        options.extra = serde_json::json!({
            "knowledge_mounts": [{
                "knowledge_base_id": "0190f5fe-7c00-7a00-8000-000000000099"
            }]
        });

        assert!(matches!(
            registry
                .get_or_create_runtime("conv-unleased-knowledge", options)
                .await,
            Err(AppError::Conflict(message))
                if message.contains("requires an exact physical workspace binding lease")
        ));
        assert_eq!(
            factory_calls.load(Ordering::SeqCst),
            0,
            "unleased knowledge metadata must be rejected before factory execution"
        );
        assert!(
            !registry.has_registered_runtime("conv-unleased-knowledge"),
            "a rejected bypass must not allocate a runtime slot"
        );
    }

    #[tokio::test]
    async fn workspace_lease_for_different_physical_directory_is_rejected_before_factory() {
        let leased_workspace = tempfile::tempdir().unwrap();
        let runtime_workspace = tempfile::tempdir().unwrap();
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&factory_calls);
        let factory: AgentRuntimeFactory = Arc::new(move |options| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(mock_runtime(MockAgent::new(
                    &options.conversation_id,
                    None,
                )))
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let mut options = make_runtime_options("conv-mismatched-workspace-lease");
        options.workspace = runtime_workspace.path().to_string_lossy().into_owned();
        options.workspace_binding_lease = Some(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                leased_workspace.path(),
                "binding-a",
                "conv-mismatched-workspace-lease",
            )
            .unwrap(),
        );

        assert!(matches!(
            registry
                .get_or_create_runtime("conv-mismatched-workspace-lease", options)
                .await,
            Err(AppError::Conflict(message))
                if message.contains("different physical workspace")
        ));
        assert_eq!(
            factory_calls.load(Ordering::SeqCst),
            0,
            "a mismatched lease must be rejected before factory execution"
        );
        assert!(
            !registry.has_registered_runtime("conv-mismatched-workspace-lease"),
            "a mismatched lease must not allocate a runtime slot"
        );
    }

    #[tokio::test]
    async fn runtime_slot_holds_workspace_binding_until_proven_teardown() {
        let workspace = tempfile::tempdir().unwrap();
        let mut options = make_runtime_options("conv-workspace-lease");
        options.workspace = workspace.path().to_string_lossy().into_owned();
        options.workspace_binding_lease = Some(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-a",
                "conv-workspace-lease",
            )
            .unwrap(),
        );
        let registry = make_registry();
        registry
            .get_or_create_runtime("conv-workspace-lease", options)
            .await
            .expect("runtime build accepts the pre-acquired lease");

        assert!(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-b",
                "other-conversation",
            )
            .is_err(),
            "dropping build options must not release a live runtime's authority"
        );

        registry
            .terminate_and_wait_result("conv-workspace-lease", None)
            .await
            .expect("mock process exit is proven");
        nomifun_knowledge::WorkspaceBindingLease::acquire(
            workspace.path(),
            "binding-b",
            "other-conversation",
        )
        .expect("a different binding can take over after exact teardown");
    }

    #[tokio::test]
    async fn failed_runtime_teardown_retains_workspace_binding_authority() {
        let workspace = tempfile::tempdir().unwrap();
        let factory: AgentRuntimeFactory = Arc::new(|options| {
            async move {
                Ok(mock_runtime(
                    MockAgent::new(
                        &options.conversation_id,
                        Some(ConversationStatus::Finished),
                    )
                    .with_kill_error("process exit was not proven"),
                ))
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let mut options = make_runtime_options("conv-workspace-quarantine");
        options.workspace = workspace.path().to_string_lossy().into_owned();
        options.workspace_binding_lease = Some(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-a",
                "conv-workspace-quarantine",
            )
            .unwrap(),
        );
        registry
            .get_or_create_runtime("conv-workspace-quarantine", options)
            .await
            .unwrap();

        assert!(
            registry
                .terminate_and_wait_result("conv-workspace-quarantine", None)
                .await
                .is_err()
        );
        assert!(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-b",
                "other-conversation",
            )
            .is_err(),
            "teardown quarantine must retain physical workspace authority"
        );
    }

    #[tokio::test]
    async fn live_runtime_without_exact_workspace_lease_cannot_be_adopted() {
        let workspace = tempfile::tempdir().unwrap();
        let registry = make_registry();
        // Simulate a corrupt/legacy in-memory slot that predates mandatory
        // workspace authority. The public registry path can no longer create
        // this state, but it must still refuse after-the-fact adoption.
        let legacy_slot = Arc::new(OnceCell::new());
        assert!(
            legacy_slot
                .set(mock_runtime(MockAgent::new("conv-unknown-binding", None)))
                .is_ok()
        );
        registry
            .runtimes
            .insert("conv-unknown-binding".to_owned(), legacy_slot);

        let mut options = make_runtime_options("conv-unknown-binding");
        options.workspace = workspace.path().to_string_lossy().into_owned();
        options.workspace_binding_lease = Some(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-a",
                "conv-unknown-binding",
            )
            .unwrap(),
        );
        assert!(matches!(
            registry
                .get_or_create_runtime("conv-unknown-binding", options)
                .await,
            Err(AppError::Conflict(message)) if message.contains("unknown knowledge workspace authority")
        ));
    }

    #[tokio::test]
    async fn cached_workspace_bound_runtime_requires_same_binding_lease_on_every_reuse() {
        let workspace = tempfile::tempdir().unwrap();
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&factory_calls);
        let factory: AgentRuntimeFactory = Arc::new(move |options| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(mock_runtime(MockAgent::new(
                    &options.conversation_id,
                    None,
                )))
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let mut initial = make_runtime_options("conv-bound-reuse");
        initial.workspace = workspace.path().to_string_lossy().into_owned();
        initial.workspace_binding_lease = Some(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-a",
                "conv-bound-reuse",
            )
            .unwrap(),
        );
        let first = registry
            .get_or_create_runtime("conv-bound-reuse", initial)
            .await
            .unwrap();

        let mut unleased = make_runtime_options("conv-bound-reuse");
        unleased.workspace = workspace.path().to_string_lossy().into_owned();
        unleased.workspace_binding_lease = None;
        assert!(matches!(
            registry
                .get_or_create_runtime("conv-bound-reuse", unleased)
                .await,
            Err(AppError::Conflict(message))
                if message.contains("requires an exact physical workspace binding lease")
        ));

        let mut same_binding = make_runtime_options("conv-bound-reuse");
        same_binding.workspace = workspace.path().to_string_lossy().into_owned();
        same_binding.workspace_binding_lease = Some(
            nomifun_knowledge::WorkspaceBindingLease::acquire(
                workspace.path(),
                "binding-a",
                "conv-bound-reuse-second-call",
            )
            .unwrap(),
        );
        let reused = registry
            .get_or_create_runtime("conv-bound-reuse", same_binding)
            .await
            .expect("the exact same binding lease may reuse the cached runtime");

        assert!(same_mock(&first, &reused));
        assert_eq!(
            factory_calls.load(Ordering::SeqCst),
            1,
            "binding validation must not rebuild a healthy exact runtime"
        );
    }

    #[tokio::test]
    async fn persisted_nomi_reset_requires_proven_runtime_exit() {
        let root = tempfile::tempdir().expect("temp root");
        let session_directory = root.path().join("nomi-sessions");
        let manager = SessionManager::new(session_directory.clone(), 100);
        let conversation_id = nomifun_common::ConversationId::new().into_string();
        let conversation_created_at = now_ms() - 1_000;
        let mut session = manager
            .create("openai", "model", "/workspace", Some(&conversation_id))
            .expect("create persisted session");
        session.owner_token = Some(conversation_created_at.to_string());
        session.messages.push(Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "old resumable context".to_owned(),
            }],
        ));
        manager.save(&session).expect("save persisted context");
        manager
            .update_index_for(&session)
            .expect("index persisted context");

        let registry = make_registry().with_nomi_session_directory(session_directory.clone());
        registry
            .get_or_create_runtime(
                &conversation_id,
                make_runtime_options(&conversation_id),
            )
            .await
            .expect("register live runtime");

        assert!(matches!(
            registry
                .reset_persisted_nomi_session(
                    &conversation_id,
                    conversation_created_at,
                )
                .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            manager
                .load(&conversation_id)
                .expect("live runtime conflict preserves context")
                .messages
                .len(),
            1
        );

        registry
            .terminate_and_wait_result(
                &conversation_id,
                Some(AgentKillReason::UserCancelled),
            )
            .await
            .expect("prove runtime exit");
        assert_eq!(
            registry
                .reset_persisted_nomi_session(
                    &conversation_id,
                    conversation_created_at,
                )
                .await
                .expect("reset after teardown"),
            NomiSessionResetOutcome::Cleared
        );
        assert!(
            SessionManager::new(session_directory, 100)
                .load(&conversation_id)
                .expect("fresh loader")
                .messages
                .is_empty()
        );
    }

    #[test]
    fn lifecycle_gate_cache_reclaims_expired_conversations_without_replacing_live_gate() {
        let registry = make_registry();
        let live = registry.lifecycle_gate("live");
        for index in 0..=MAX_CACHED_LIFECYCLE_GATES {
            drop(registry.lifecycle_gate(&format!("expired-{index}")));
        }

        let same_live = registry.lifecycle_gate("live");
        assert!(Arc::ptr_eq(&live, &same_live));
        assert!(registry.lifecycle_gates.len() < MAX_CACHED_LIFECYCLE_GATES);
    }

    #[tokio::test]
    async fn get_or_create_creates_runtime() {
        let registry = make_registry();
        let runtime = registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        assert_eq!(runtime.conversation_id(), "conv-1");
        assert_eq!(registry.active_runtime_count(), 1);
    }

    #[tokio::test]
    async fn get_or_create_returns_existing() {
        let registry = make_registry();
        let h1 = registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        let h2 = registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        assert!(same_mock(&h1, &h2));
        assert_eq!(registry.active_runtime_count(), 1);
    }

    #[tokio::test]
    async fn get_or_create_is_single_flight_under_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_factory = Arc::clone(&calls);
        let factory: AgentRuntimeFactory = Arc::new(move |opts: AgentRuntimeBuildOptions| {
            let calls = Arc::clone(&calls_for_factory);
            async move {
                // Simulate a slow build (CLI spawn + initialize handshake).
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(mock_runtime(MockAgent::new(&opts.conversation_id, None)))
            }
            .boxed()
        });
        let registry = Arc::new(InMemoryAgentRuntimeRegistry::new(factory));

        // Ten concurrent callers all racing on the same conversation id.
        let mut joins = Vec::new();
        for _ in 0..10 {
            let registry = Arc::clone(&registry);
            joins.push(tokio::spawn(async move {
                registry.get_or_create_runtime("conv-race", make_runtime_options("conv-race")).await
            }));
        }
        let handles: Vec<_> = futures_util::future::join_all(joins)
            .await
            .into_iter()
            .map(|r| r.unwrap().unwrap())
            .collect();

        assert_eq!(calls.load(Ordering::SeqCst), 1, "factory must run only once");
        assert_eq!(registry.active_runtime_count(), 1);
        for h in handles.iter().skip(1) {
            assert!(same_mock(&handles[0], h), "all callers see the same handle");
        }
    }

    #[tokio::test]
    async fn get_or_create_retries_after_failure() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let fail_next = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&fail_next);
        let factory: AgentRuntimeFactory = Arc::new(move |opts: AgentRuntimeBuildOptions| {
            let flag = Arc::clone(&flag);
            async move {
                if flag.swap(false, Ordering::SeqCst) {
                    Err(AppError::Internal("first call fails".into()))
                } else {
                    Ok(mock_runtime(MockAgent::new(&opts.conversation_id, None)))
                }
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);

        // First call fails, slot stays empty.
        assert!(registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.is_err());
        // Second call retries and succeeds — the FIRST build failure never
        // imposes a cooldown (manual user retries stay free).
        let h = registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        assert_eq!(h.conversation_id(), "conv-1");
        assert_eq!(registry.active_runtime_count(), 1);
    }

    /// From the second consecutive build failure onward, further builds are
    /// refused fast (Conflict) during an escalating cooldown instead of
    /// re-running the factory — the pacing that stops automated senders from
    /// burning a full CLI spawn + handshake timeout per message when the
    /// agent cannot start at all. A success clears the streak.
    #[tokio::test]
    async fn build_failure_streak_cools_down_then_recovers() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_factory = Arc::clone(&calls);
        // Fails twice, then succeeds.
        let factory: AgentRuntimeFactory = Arc::new(move |opts: AgentRuntimeBuildOptions| {
            let calls = Arc::clone(&calls_for_factory);
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(AppError::BadGateway("initialize handshake timed out".into()))
                } else {
                    Ok(mock_runtime(MockAgent::new(&opts.conversation_id, None)))
                }
            }
            .boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);

        // Failure #1: factory ran, no cooldown yet.
        assert!(registry.get_or_create_runtime("conv-cd", make_runtime_options("conv-cd")).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Failure #2: factory ran again (first failure is retry-free).
        assert!(registry.get_or_create_runtime("conv-cd", make_runtime_options("conv-cd")).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Attempt #3 within the cooldown: refused FAST, factory NOT run,
        // error names the cooldown.
        let Err(refused) = registry
            .get_or_create_runtime("conv-cd", make_runtime_options("conv-cd"))
            .await
        else {
            panic!("cooldown must refuse the third immediate attempt");
        };
        assert_eq!(calls.load(Ordering::SeqCst), 2, "factory must not run during cooldown");
        assert!(
            refused.to_string().contains("cooling down"),
            "refusal must explain the cooldown, got: {refused}"
        );

        // After the cooldown elapses the build runs and succeeds; the streak
        // is cleared so a later failure starts over at "retry-free".
        registry
            .governor
            .build_failures
            .get_mut("conv-cd")
            .expect("streak record present")
            .last_failure_ms -= BUILD_FAILURE_COOLDOWN_BASE_MS + 1;
        let h = registry
            .get_or_create_runtime("conv-cd", make_runtime_options("conv-cd"))
            .await
            .expect("build must proceed after the cooldown");
        assert_eq!(h.conversation_id(), "conv-cd");
        assert!(
            registry.governor.build_failures.get("conv-cd").is_none(),
            "a successful build must clear the failure streak"
        );
    }

    #[test]
    fn build_failure_governor_escalates_and_resets() {
        let governor = RestartGovernor::default();
        let t0 = 1_000_000i64;

        // No record → free.
        assert!(governor.build_gate("c", t0).is_ok());

        // Failure #1 → still free.
        assert_eq!(governor.record_build_failure("c", t0), 1);
        assert!(governor.build_gate("c", t0 + 1).is_ok());

        // Failure #2 → 30s cooldown.
        assert_eq!(governor.record_build_failure("c", t0 + 10), 2);
        let remaining = governor.build_gate("c", t0 + 10 + 1).expect_err("cooldown engaged");
        assert!(remaining > 0 && remaining <= BUILD_FAILURE_COOLDOWN_BASE_MS);
        assert!(governor.build_gate("c", t0 + 10 + BUILD_FAILURE_COOLDOWN_BASE_MS).is_ok());

        // Failure #3 → 60s; failure #4 → 120s (capped thereafter).
        assert_eq!(governor.record_build_failure("c", t0 + 100), 3);
        let remaining = governor.build_gate("c", t0 + 100 + 1).expect_err("cooldown engaged");
        assert!(remaining > BUILD_FAILURE_COOLDOWN_BASE_MS && remaining <= 2 * BUILD_FAILURE_COOLDOWN_BASE_MS);
        assert_eq!(governor.record_build_failure("c", t0 + 200), 4);
        let remaining = governor.build_gate("c", t0 + 200 + 1).expect_err("cooldown engaged");
        assert!(remaining <= BUILD_FAILURE_COOLDOWN_MAX_MS);
        assert_eq!(governor.record_build_failure("c", t0 + 300), 5);
        let remaining = governor.build_gate("c", t0 + 300 + 1).expect_err("cooldown engaged");
        assert!(remaining <= BUILD_FAILURE_COOLDOWN_MAX_MS, "cooldown must cap");

        // A quiet period longer than the reset window forgets the streak.
        let later = t0 + 300 + BUILD_FAILURE_RESET_MS + 1;
        assert!(governor.build_gate("c", later).is_ok());
        assert_eq!(governor.record_build_failure("c", later), 1, "streak restarts after reset window");

        // Success clears; forget clears.
        governor.record_build_success("c");
        assert!(governor.build_failures.get("c").is_none());
        governor.record_build_failure("c", later);
        governor.forget("c");
        assert!(governor.build_failures.get("c").is_none());
    }

    #[tokio::test]
    async fn synchronous_terminate_during_initialization_does_not_leak_runtime() {
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let factory: AgentRuntimeFactory = {
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            Arc::new(move |options: AgentRuntimeBuildOptions| {
                let started = Arc::clone(&started);
                let release = Arc::clone(&release);
                async move {
                    started.add_permits(1);
                    let _permit = release.acquire().await.expect("release semaphore remains open");
                    Ok(mock_runtime(MockAgent::new(&options.conversation_id, None)))
                }
                .boxed()
            })
        };
        let registry = Arc::new(InMemoryAgentRuntimeRegistry::new(factory));
        let build = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .get_or_create_runtime("conv-init-race", make_runtime_options("conv-init-race"))
                    .await
            })
        };

        started.acquire().await.expect("factory starts").forget();
        registry.terminate("conv-init-race", None).expect("termination succeeds");
        release.add_permits(1);

        let result = build.await.expect("build task joins");
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert!(registry.get_runtime("conv-init-race").is_none());
        assert_eq!(registry.active_runtime_count(), 0);
    }

    #[tokio::test]
    async fn get_runtime_finds_existing() {
        let registry = make_registry();
        registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        let handle = registry.get_runtime("conv-1");
        assert!(handle.is_some());
        assert_eq!(handle.unwrap().conversation_id(), "conv-1");
    }

    #[tokio::test]
    async fn next_turn_rebuilds_instead_of_reusing_a_runtime_with_a_dead_relay() {
        use std::sync::atomic::AtomicUsize;

        let calls = Arc::new(AtomicUsize::new(0));
        let first_health = Arc::new(AtomicBool::new(true));
        let factory: AgentRuntimeFactory = {
            let calls = Arc::clone(&calls);
            let first_health = Arc::clone(&first_health);
            Arc::new(move |options: AgentRuntimeBuildOptions| {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                let agent = if call == 0 {
                    MockAgent::new(&options.conversation_id, Some(ConversationStatus::Finished))
                        .with_transport_health(Arc::clone(&first_health))
                } else {
                    MockAgent::new(&options.conversation_id, None)
                };
                async move { Ok(mock_runtime(agent)) }.boxed()
            })
        };
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let first = registry
            .get_or_create_runtime("conv-broken-relay", make_runtime_options("conv-broken-relay"))
            .await
            .expect("initial runtime");
        first_health.store(false, Ordering::Release);

        assert!(
            registry.get_runtime("conv-broken-relay").is_none(),
            "read-only lookup must not expose a dead cached transport",
        );
        let replacement = registry
            .get_or_create_runtime("conv-broken-relay", make_runtime_options("conv-broken-relay"))
            .await
            .expect("dead relay is evicted and rebuilt");
        assert!(!same_mock(&first, &replacement));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(replacement.is_transport_healthy());
    }

    #[tokio::test]
    async fn stateless_successor_waits_for_exact_old_process_exit_before_rebuild() {
        let calls = Arc::new(AtomicUsize::new(0));
        let recycle_required = Arc::new(AtomicBool::new(false));
        let kill_started = Arc::new(Semaphore::new(0));
        let kill_release = Arc::new(Semaphore::new(0));
        let kill_reasons = Arc::new(std::sync::Mutex::new(Vec::new()));
        let factory: AgentRuntimeFactory = {
            let calls = Arc::clone(&calls);
            let recycle_required = Arc::clone(&recycle_required);
            let kill_started = Arc::clone(&kill_started);
            let kill_release = Arc::clone(&kill_release);
            let kill_reasons = Arc::clone(&kill_reasons);
            Arc::new(move |options: AgentRuntimeBuildOptions| {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                let agent = if call == 0 {
                    MockAgent::new(
                        &options.conversation_id,
                        Some(ConversationStatus::Finished),
                    )
                    .with_agent_type(AgentType::Nanobot)
                    .with_turn_boundary_recycle(Arc::clone(&recycle_required))
                    .with_blocking_kill(
                        Arc::clone(&kill_started),
                        Arc::clone(&kill_release),
                    )
                    .with_kill_reasons(Arc::clone(&kill_reasons))
                } else {
                    MockAgent::new(&options.conversation_id, None)
                        .with_agent_type(AgentType::Nanobot)
                };
                async move { Ok(mock_runtime(agent)) }.boxed()
            })
        };
        let registry = Arc::new(InMemoryAgentRuntimeRegistry::new(factory));
        let first = registry
            .get_or_create_runtime_for_turn(
                "conv-stateless-fence",
                1,
                CancellationToken::new(),
                make_runtime_options("conv-stateless-fence"),
            )
            .await
            .unwrap();
        recycle_required.store(true, Ordering::Release);
        registry
            .release_runtime_turn("conv-stateless-fence", 1)
            .await
            .unwrap();

        let successor = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .get_or_create_runtime_for_turn(
                        "conv-stateless-fence",
                        2,
                        CancellationToken::new(),
                        make_runtime_options("conv-stateless-fence"),
                    )
                    .await
            })
        };
        kill_started.acquire().await.unwrap().forget();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "successor factory must remain closed until exact old-process exit"
        );
        assert!(!successor.is_finished());

        kill_release.add_permits(1);
        let replacement = successor.await.unwrap().unwrap();
        assert!(!same_mock(&first, &replacement));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            *kill_reasons.lock().unwrap(),
            vec![Some(AgentKillReason::TurnBoundaryRecycle)]
        );
    }

    #[tokio::test]
    async fn repeated_normal_stateless_turn_recycles_never_trip_crash_governor() {
        const TURN_COUNT: u64 = 6;
        let calls = Arc::new(AtomicUsize::new(0));
        let recycle_flags = Arc::new(std::sync::Mutex::new(Vec::<Arc<AtomicBool>>::new()));
        let kill_reasons = Arc::new(std::sync::Mutex::new(Vec::new()));
        let factory: AgentRuntimeFactory = {
            let calls = Arc::clone(&calls);
            let recycle_flags = Arc::clone(&recycle_flags);
            let kill_reasons = Arc::clone(&kill_reasons);
            Arc::new(move |options: AgentRuntimeBuildOptions| {
                calls.fetch_add(1, Ordering::SeqCst);
                let recycle_required = Arc::new(AtomicBool::new(false));
                recycle_flags
                    .lock()
                    .unwrap()
                    .push(Arc::clone(&recycle_required));
                let agent = MockAgent::new(
                    &options.conversation_id,
                    Some(ConversationStatus::Finished),
                )
                .with_agent_type(AgentType::Nanobot)
                .with_turn_boundary_recycle(recycle_required)
                .with_kill_reasons(Arc::clone(&kill_reasons));
                async move { Ok(mock_runtime(agent)) }.boxed()
            })
        };
        let registry = InMemoryAgentRuntimeRegistry::new(factory);

        for generation in 1..=TURN_COUNT {
            registry
                .get_or_create_runtime_for_turn(
                    "conv-stateless-repeat",
                    generation,
                    CancellationToken::new(),
                    make_runtime_options("conv-stateless-repeat"),
                )
                .await
                .unwrap_or_else(|error| {
                    panic!("normal stateless turn {generation} was throttled: {error}")
                });
            let flag = recycle_flags.lock().unwrap()[(generation - 1) as usize].clone();
            flag.store(true, Ordering::Release);
            registry
                .release_runtime_turn("conv-stateless-repeat", generation)
                .await
                .unwrap();
        }

        // One more admission forces teardown of the sixth completed process.
        registry
            .get_or_create_runtime_for_turn(
                "conv-stateless-repeat",
                TURN_COUNT + 1,
                CancellationToken::new(),
                make_runtime_options("conv-stateless-repeat"),
            )
            .await
            .expect("normal boundary recycles must remain admissible beyond crash limit");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            (TURN_COUNT + 1) as usize
        );
        assert_eq!(
            kill_reasons.lock().unwrap().as_slice(),
            vec![Some(AgentKillReason::TurnBoundaryRecycle); TURN_COUNT as usize]
        );
        assert_eq!(
            registry
                .governor
                .gate("conv-stateless-repeat", now_ms()),
            Ok(0),
            "normal turn-boundary recycle must not consume crash budget"
        );
    }

    #[tokio::test]
    async fn terminate_removes_runtime() {
        let registry = make_registry();
        registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        assert_eq!(registry.active_runtime_count(), 1);

        registry.terminate("conv-1", Some(AgentKillReason::IdleTimeout)).unwrap();
        assert_eq!(registry.active_runtime_count(), 0);
        assert!(registry.get_runtime("conv-1").is_none());
    }

    #[tokio::test]
    async fn terminate_and_wait_blocks_recreate_until_old_agent_finishes() {
        use std::sync::atomic::AtomicUsize;

        let calls = Arc::new(AtomicUsize::new(0));
        let kill_started = Arc::new(Semaphore::new(0));
        let kill_release = Arc::new(Semaphore::new(0));
        let factory: AgentRuntimeFactory = {
            let calls = Arc::clone(&calls);
            let kill_started = Arc::clone(&kill_started);
            let kill_release = Arc::clone(&kill_release);
            Arc::new(move |opts: AgentRuntimeBuildOptions| {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                let agent = if call == 0 {
                    MockAgent::new(&opts.conversation_id, Some(ConversationStatus::Running))
                        .with_blocking_kill(Arc::clone(&kill_started), Arc::clone(&kill_release))
                } else {
                    MockAgent::new(&opts.conversation_id, None)
                };
                async move { Ok(mock_runtime(agent)) }.boxed()
            })
        };
        let registry = Arc::new(InMemoryAgentRuntimeRegistry::new(factory));
        registry.get_or_create_runtime("conv-teardown", make_runtime_options("conv-teardown"))
            .await
            .unwrap();

        let teardown = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                let _ = registry.terminate_and_wait_result("conv-teardown", None).await;
            })
        };
        kill_started.acquire().await.unwrap().forget();

        let rebuild = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry.get_or_create_runtime("conv-teardown", make_runtime_options("conv-teardown"))
                    .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "factory must wait behind teardown");
        assert!(!rebuild.is_finished());

        kill_release.add_permits(1);
        teardown.await.unwrap();
        rebuild.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn terminate_nonexistent_is_ok() {
        let factory: AgentRuntimeFactory = Arc::new(|_| async { unreachable!() }.boxed());
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        assert!(registry.terminate("nothing", None).is_ok());
    }

    #[tokio::test]
    async fn terminate_all_removes_all() {
        let registry = make_registry();
        registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        registry.get_or_create_runtime("conv-2", make_runtime_options("conv-2")).await.unwrap();
        assert_eq!(registry.active_runtime_count(), 2);

        registry.terminate_all();
        assert_eq!(registry.active_runtime_count(), 0);
    }

    #[test]
    fn collect_idle_finds_finished_and_stale_acp_runtimes() {
        let factory: AgentRuntimeFactory = Arc::new(|_| async { unreachable!() }.boxed());
        let registry = InMemoryAgentRuntimeRegistry::new(factory);

        // Helper: insert a pre-initialised slot bypassing the async factory path.
        let insert = |id: &str, runtime: AgentRuntimeHandle| {
            let cell: OnceCell<AgentRuntimeHandle> = OnceCell::new();
            cell.set(runtime).ok();
            registry.runtimes.insert(id.into(), Arc::new(cell));
        };

        // ACP + Finished + old activity → should be collected
        insert(
            "conv-stale",
            mock_runtime(
                MockAgent::new("conv-stale", Some(ConversationStatus::Finished)).with_last_activity(now_ms() - 600_000),
            ),
        );

        // ACP + Finished + recent activity → should NOT be collected
        insert(
            "conv-recent",
            mock_runtime(
                MockAgent::new("conv-recent", Some(ConversationStatus::Finished)).with_last_activity(now_ms()),
            ),
        );

        // ACP + Running + old activity → should NOT be collected
        insert(
            "conv-running",
            mock_runtime(
                MockAgent::new("conv-running", Some(ConversationStatus::Running))
                    .with_last_activity(now_ms() - 600_000),
            ),
        );

        // Non-ACP (Nanobot) + Finished + old activity → should NOT be collected
        insert(
            "conv-nanobot",
            mock_runtime(
                MockAgent::new("conv-nanobot", Some(ConversationStatus::Finished))
                    .with_agent_type(AgentType::Nanobot)
                    .with_last_activity(now_ms() - 600_000),
            ),
        );

        let idle = registry.collect_idle_runtimes(300_000); // 5-min threshold
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0], "conv-stale");
    }

    #[test]
    fn collect_idle_empty_when_no_runtimes() {
        let registry = make_registry();
        let idle = registry.collect_idle_runtimes(300_000);
        assert!(idle.is_empty());
    }

    #[tokio::test]
    async fn idle_termination_removes_runtime_only_after_revalidation() {
        let factory: AgentRuntimeFactory = Arc::new(|_| async { unreachable!() }.boxed());
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let workspace = tempfile::tempdir().unwrap();
        let lease = nomifun_knowledge::WorkspaceBindingLease::acquire(
            workspace.path(),
            "binding-idle-a",
            "conv-idle",
        )
        .unwrap();
        let runtime = mock_runtime(
            MockAgent::new("conv-idle", Some(ConversationStatus::Finished))
                .with_last_activity(now_ms() - 600_000),
        );
        let cell: OnceCell<AgentRuntimeHandle> = OnceCell::new();
        cell.set(runtime).ok().expect("fresh runtime cell");
        let slot = Arc::new(cell);
        registry
            .runtimes
            .insert("conv-idle".into(), Arc::clone(&slot));
        registry.workspace_bindings.insert(
            "conv-idle".into(),
            RuntimeWorkspaceBinding { slot, lease },
        );

        assert!(
            registry
                .terminate_idle_runtime_if_eligible("conv-idle", 300_000)
                .await
                .expect("idle teardown succeeds")
        );
        assert!(
            registry.get_runtime("conv-idle").is_none(),
            "a runtime that is still idle under the lifecycle gate is removed"
        );
        nomifun_knowledge::WorkspaceBindingLease::acquire(
            workspace.path(),
            "binding-idle-b",
            "next-conversation",
        )
        .expect("proven idle teardown releases physical workspace authority");
    }

    #[tokio::test]
    async fn admitted_finished_runtime_is_not_idle_collectable_until_exact_release() {
        let factory: AgentRuntimeFactory = Arc::new(|_| async { unreachable!() }.boxed());
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let workspace = tempfile::tempdir().unwrap();
        let lease = nomifun_knowledge::WorkspaceBindingLease::acquire(
            workspace.path(),
            "binding-idle-admitted",
            "conv-idle-admitted",
        )
        .unwrap();
        let runtime = mock_runtime(
            MockAgent::new(
                "conv-idle-admitted",
                Some(ConversationStatus::Finished),
            )
            .with_last_activity(now_ms() - 600_000),
        );
        let cell: OnceCell<AgentRuntimeHandle> = OnceCell::new();
        cell.set(runtime).ok().expect("fresh runtime cell");
        let slot = Arc::new(cell);
        registry
            .runtimes
            .insert("conv-idle-admitted".into(), Arc::clone(&slot));
        registry.workspace_bindings.insert(
            "conv-idle-admitted".into(),
            RuntimeWorkspaceBinding {
                slot: Arc::clone(&slot),
                lease,
            },
        );
        registry
            .bind_turn_admission_exact("conv-idle-admitted", 17, &slot)
            .expect("test turn admission");

        assert!(
            registry.collect_idle_runtimes(300_000).is_empty(),
            "scanner candidates must exclude exact admitted slots"
        );
        assert!(
            !registry
                .terminate_idle_runtime_if_eligible("conv-idle-admitted", 300_000)
                .await
                .expect("idle revalidation succeeds"),
            "Finished/activity cannot override an unreleased turn admission"
        );
        assert!(registry.get_runtime("conv-idle-admitted").is_some());

        registry
            .release_runtime_turn("conv-idle-admitted", 17)
            .await
            .expect("durable/local terminal boundary releases exact admission");
        assert!(
            registry
                .terminate_idle_runtime_if_eligible("conv-idle-admitted", 300_000)
                .await
                .expect("idle teardown succeeds after release"),
            "the same stale Finished runtime becomes eligible only after exact release"
        );
        assert!(registry.get_runtime("conv-idle-admitted").is_none());
    }

    #[tokio::test]
    async fn turn_admission_refreshes_finished_runtime_before_idle_revalidation() {
        let factory: AgentRuntimeFactory = Arc::new(|_| async { unreachable!() }.boxed());
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let stale_activity = now_ms() - 600_000;
        let runtime = mock_runtime(
            MockAgent::new("conv-reactivated", Some(ConversationStatus::Finished))
                .with_last_activity(stale_activity),
        );
        let cell: OnceCell<AgentRuntimeHandle> = OnceCell::new();
        cell.set(runtime.clone()).ok().expect("fresh runtime cell");
        let slot = Arc::new(cell);
        let options = make_runtime_options("conv-reactivated");
        let lease = options
            .workspace_binding_lease
            .as_ref()
            .expect("turn options carry workspace authority")
            .clone();
        registry
            .runtimes
            .insert("conv-reactivated".into(), Arc::clone(&slot));
        registry.workspace_bindings.insert(
            "conv-reactivated".into(),
            RuntimeWorkspaceBinding { slot, lease },
        );

        // The scanner first records a stale id. Turn admission then returns the
        // existing Finished runtime; `send_message` has not yet changed its
        // status, so only the gate-protected activity touch closes this gap.
        assert_eq!(
            registry.collect_idle_runtimes(300_000),
            vec!["conv-reactivated".to_owned()]
        );
        let admitted = registry
            .get_or_create_runtime_for_turn(
                "conv-reactivated",
                7,
                CancellationToken::new(),
                options,
            )
            .await
            .expect("turn admission reuses the healthy runtime");

        assert_eq!(admitted.status(), Some(ConversationStatus::Finished));
        assert!(
            admitted.last_activity_at() > stale_activity,
            "turn admission must refresh activity before releasing the lifecycle gate"
        );
        assert!(
            !registry
                .terminate_idle_runtime_if_eligible("conv-reactivated", 300_000)
                .await
                .expect("idle revalidation succeeds"),
            "the stale scan result must be rejected even before send_message marks Running"
        );
        let current = registry
            .get_runtime("conv-reactivated")
            .expect("turn-admitted runtime remains registered");
        assert!(same_mock(&runtime, &current));
    }

    #[tokio::test]
    async fn stale_idle_scan_cannot_terminate_replacement_runtime() {
        let factory: AgentRuntimeFactory = Arc::new(|_| async { unreachable!() }.boxed());
        let registry = InMemoryAgentRuntimeRegistry::new(factory);

        let old_runtime = mock_runtime(
            MockAgent::new("conv-replaced", Some(ConversationStatus::Finished))
                .with_last_activity(now_ms() - 600_000),
        );
        let old_cell: OnceCell<AgentRuntimeHandle> = OnceCell::new();
        old_cell
            .set(old_runtime)
            .ok()
            .expect("fresh old runtime cell");
        registry
            .runtimes
            .insert("conv-replaced".into(), Arc::new(old_cell));
        assert_eq!(
            registry.collect_idle_runtimes(300_000),
            vec!["conv-replaced".to_owned()]
        );

        registry
            .terminate("conv-replaced", None)
            .expect("old runtime is removed");
        let replacement = mock_runtime(MockAgent::new(
            "conv-replaced",
            Some(ConversationStatus::Running),
        ));
        let replacement_cell: OnceCell<AgentRuntimeHandle> = OnceCell::new();
        replacement_cell
            .set(replacement.clone())
            .ok()
            .expect("fresh replacement runtime cell");
        registry
            .runtimes
            .insert("conv-replaced".into(), Arc::new(replacement_cell));

        assert!(
            !registry
                .terminate_idle_runtime_if_eligible("conv-replaced", 300_000)
                .await
                .expect("idle revalidation succeeds"),
            "a conversation-id snapshot cannot authorize terminating its replacement"
        );
        let current = registry
            .get_runtime("conv-replaced")
            .expect("replacement runtime remains registered");
        assert!(same_mock(&replacement, &current));
    }

    #[tokio::test]
    async fn failed_idle_teardown_quarantines_exact_slot_and_blocks_replacement() {
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&factory_calls);
        let factory: AgentRuntimeFactory = Arc::new(move |options| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { Ok(mock_runtime(MockAgent::new(&options.conversation_id, None))) }.boxed()
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
        let runtime = mock_runtime(
            MockAgent::new("conv-kill-fails", Some(ConversationStatus::Finished))
                .with_last_activity(now_ms() - 600_000)
                .with_kill_error("deterministic process-tree kill failure"),
        );
        let slot: RuntimeSlot = {
            let cell = OnceCell::new();
            cell.set(runtime).ok().expect("fresh runtime cell");
            Arc::new(cell)
        };
        registry
            .runtimes
            .insert("conv-kill-fails".into(), Arc::clone(&slot));

        let first_cleanup = registry
            .terminate_idle_runtime_if_eligible("conv-kill-fails", 300_000)
            .await;
        assert!(matches!(first_cleanup, Err(AppError::Internal(message)) if message.contains("kill failure")));
        assert!(
            registry.get_runtime("conv-kill-fails").is_none(),
            "a failed teardown slot must not be exposed for reuse"
        );
        assert!(
            registry.has_registered_runtime("conv-kill-fails"),
            "quarantined ownership must remain visible to orphan reconciliation"
        );
        assert!(registry.slot_is_quarantined("conv-kill-fails", &slot));
        let retained = registry
            .runtimes
            .get("conv-kill-fails")
            .expect("failed slot remains authoritative");
        assert!(Arc::ptr_eq(retained.value(), &slot));
        drop(retained);

        let retry = registry
            .get_or_create_runtime(
                "conv-kill-fails",
                make_runtime_options("conv-kill-fails"),
            )
            .await;
        assert!(
            matches!(retry, Err(AppError::Internal(message)) if message.contains("kill failure")),
            "admission must retry and propagate the same teardown failure"
        );
        assert_eq!(
            factory_calls.load(Ordering::SeqCst),
            0,
            "replacement factory must stay blocked until old process exit is proven"
        );
        assert!(registry.slot_is_quarantined("conv-kill-fails", &slot));
    }

    #[tokio::test]
    async fn active_turn_generation_cannot_be_overwritten_and_stale_release_is_absorbing() {
        let registry = InMemoryAgentRuntimeRegistry::new(Arc::new(|options| {
            async move {
                Ok(mock_runtime(MockAgent::new(
                    &options.conversation_id,
                    Some(ConversationStatus::Finished),
                )))
            }
            .boxed()
        }));
        let first = registry
            .get_or_create_runtime_for_turn(
                "conv-generation",
                41,
                CancellationToken::new(),
                make_runtime_options("conv-generation"),
            )
            .await
            .expect("first turn admission");
        let conflicting = registry
            .get_or_create_runtime_for_turn(
                "conv-generation",
                42,
                CancellationToken::new(),
                make_runtime_options("conv-generation"),
            )
            .await;
        assert!(
            matches!(conflicting, Err(AppError::Conflict(_))),
            "a different generation must not overwrite an active admission"
        );
        let active = registry
            .turn_admissions
            .get("conv-generation")
            .expect("first generation remains authoritative")
            .value()
            .clone();
        assert_eq!(active.generation, 41);

        registry
            .release_runtime_turn("conv-generation", 41)
            .await
            .expect("exact completed generation releases its admission");
        let current = registry
            .get_or_create_runtime_for_turn(
                "conv-generation",
                42,
                CancellationToken::new(),
                make_runtime_options("conv-generation"),
            )
            .await
            .expect("newer turn admission");
        assert!(same_mock(&first, &current));

        registry
            .release_runtime_turn("conv-generation", 41)
            .await
            .expect("stale release is an absorbing no-op");
        assert_eq!(
            registry
                .turn_admissions
                .get("conv-generation")
                .expect("successor admission survives stale release")
                .generation,
            42
        );

        registry
            .cancel_runtime_turn(
                "conv-generation",
                41,
                Some(AgentKillReason::UserCancelled),
            )
            .expect("stale cancellation is an absorbing no-op");
        let after_stale_cancel = registry
            .get_runtime("conv-generation")
            .expect("current generation runtime survives");
        assert!(same_mock(&current, &after_stale_cancel));
        assert!(registry.quarantined_slot("conv-generation").is_none());

        registry
            .cancel_runtime_turn(
                "conv-generation",
                42,
                Some(AgentKillReason::UserCancelled),
            )
            .expect("current cancellation quarantines its exact runtime");
        assert!(registry.get_runtime("conv-generation").is_none());
        registry
            .terminate_and_wait_result(
                "conv-generation",
                Some(AgentKillReason::UserCancelled),
            )
            .await
            .expect("quarantined runtime shutdown is proven");
        assert!(registry.quarantined_slot("conv-generation").is_none());
    }

    #[tokio::test]
    async fn preparation_during_active_turn_cannot_overwrite_exact_cancel_authority() {
        let registry = InMemoryAgentRuntimeRegistry::new(Arc::new(|options| {
            async move {
                Ok(mock_runtime(MockAgent::new(
                    &options.conversation_id,
                    Some(ConversationStatus::Running),
                )))
            }
            .boxed()
        }));
        let admitted = registry
            .get_or_create_runtime_for_turn(
                "conv-active-preparation",
                73,
                CancellationToken::new(),
                make_runtime_options("conv-active-preparation"),
            )
            .await
            .expect("active turn owns its exact runtime slot");

        let prepared = registry
            .get_or_create_runtime_for_preparation(
                "conv-active-preparation",
                CancellationToken::new(),
                make_runtime_options("conv-active-preparation"),
            )
            .await
            .expect("background preparation may reuse the live runtime");
        assert!(same_mock(&admitted, &prepared));
        assert_eq!(
            registry
                .turn_admissions
                .get("conv-active-preparation")
                .expect("preparation preserves active turn authority")
                .generation,
            73
        );

        registry
            .cancel_runtime_turn(
                "conv-active-preparation",
                73,
                Some(AgentKillReason::UserCancelled),
            )
            .expect("exact turn cancellation still targets the admitted slot");
        assert!(
            registry.get_runtime("conv-active-preparation").is_none(),
            "the exact active runtime is quarantined after cancellation"
        );
        assert!(registry
            .quarantined_slot("conv-active-preparation")
            .is_some());
    }

    #[tokio::test]
    async fn preparation_only_warmup_never_creates_turn_admission() {
        let registry = make_registry();
        let warmed = registry
            .get_or_create_runtime_for_preparation(
                "conv-warmup-only",
                CancellationToken::new(),
                make_runtime_options("conv-warmup-only"),
            )
            .await
            .expect("warmup builds the reusable runtime");
        assert!(
            registry.turn_admissions.get("conv-warmup-only").is_none(),
            "preparation-only warmup must not mint turn authority"
        );

        registry
            .cancel_runtime_turn(
                "conv-warmup-only",
                999,
                Some(AgentKillReason::UserCancelled),
            )
            .expect("a nonexistent turn cancellation is an absorbing no-op");
        let still_warm = registry
            .get_runtime("conv-warmup-only")
            .expect("warm runtime survives unrelated turn cancellation");
        assert!(same_mock(&warmed, &still_warm));
    }

    #[tokio::test]
    async fn warmup_then_explicit_turn_reuses_slot_and_only_turn_binds_generation() {
        let registry = make_registry();
        let warmed = registry
            .get_or_create_runtime_for_preparation(
                "conv-warm-then-send",
                CancellationToken::new(),
                make_runtime_options("conv-warm-then-send"),
            )
            .await
            .expect("view warmup prepares a runtime");
        assert!(registry
            .turn_admissions
            .get("conv-warm-then-send")
            .is_none());

        let admitted = registry
            .get_or_create_runtime_for_turn(
                "conv-warm-then-send",
                101,
                CancellationToken::new(),
                make_runtime_options("conv-warm-then-send"),
            )
            .await
            .expect("the explicit send binds the prepared slot");
        assert!(same_mock(&warmed, &admitted));
        assert_eq!(
            registry
                .turn_admissions
                .get("conv-warm-then-send")
                .expect("explicit send owns the only turn admission")
                .generation,
            101
        );
    }

    #[tokio::test]
    async fn two_completed_turns_reuse_runtime_after_exact_release() {
        let registry = make_registry();
        let first = registry
            .get_or_create_runtime_for_turn(
                "conv-two-turns",
                201,
                CancellationToken::new(),
                make_runtime_options("conv-two-turns"),
            )
            .await
            .expect("first turn admission");
        registry
            .release_runtime_turn("conv-two-turns", 201)
            .await
            .expect("first terminal boundary releases only its admission");

        let second = registry
            .get_or_create_runtime_for_turn(
                "conv-two-turns",
                202,
                CancellationToken::new(),
                make_runtime_options("conv-two-turns"),
            )
            .await
            .expect("second turn admission");
        assert!(
            same_mock(&first, &second),
            "normal completion keeps the reusable runtime slot alive"
        );
        assert_eq!(
            registry
                .turn_admissions
                .get("conv-two-turns")
                .expect("second generation is authoritative")
                .generation,
            202
        );
    }

    #[tokio::test]
    async fn turn_cancel_quarantine_blocks_replacement_until_teardown_is_proven() {
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let kill_started = Arc::new(Semaphore::new(0));
        let kill_release = Arc::new(Semaphore::new(0));
        let factory: AgentRuntimeFactory = {
            let factory_calls = Arc::clone(&factory_calls);
            let kill_started = Arc::clone(&kill_started);
            let kill_release = Arc::clone(&kill_release);
            Arc::new(move |options| {
                let call = factory_calls.fetch_add(1, Ordering::SeqCst);
                let agent = if call == 0 {
                    MockAgent::new(
                        &options.conversation_id,
                        Some(ConversationStatus::Finished),
                    )
                    .with_blocking_kill(Arc::clone(&kill_started), Arc::clone(&kill_release))
                } else {
                    MockAgent::new(&options.conversation_id, None)
                };
                async move { Ok(mock_runtime(agent)) }.boxed()
            })
        };
        let registry = Arc::new(InMemoryAgentRuntimeRegistry::new(factory));
        registry
            .get_or_create_runtime_for_turn(
                "conv-cancel-fence",
                1,
                CancellationToken::new(),
                make_runtime_options("conv-cancel-fence"),
            )
            .await
            .expect("first turn admission");
        registry
            .cancel_runtime_turn(
                "conv-cancel-fence",
                1,
                Some(AgentKillReason::UserCancelled),
            )
            .expect("exact generation is quarantined");

        let replacement = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .get_or_create_runtime_for_turn(
                        "conv-cancel-fence",
                        2,
                        CancellationToken::new(),
                        make_runtime_options("conv-cancel-fence"),
                    )
                    .await
            })
        };
        kill_started
            .acquire()
            .await
            .expect("replacement admission retries exact teardown")
            .forget();
        assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
        assert!(
            !replacement.is_finished(),
            "replacement must remain blocked while old teardown is unproven"
        );

        kill_release.add_permits(1);
        replacement
            .await
            .expect("replacement task joins")
            .expect("replacement is admitted only after proven teardown");
        assert_eq!(factory_calls.load(Ordering::SeqCst), 2);
    }

    // ── Restart governor (crash-loop breaker) ───────────────────────

    #[test]
    fn restart_governor_trips_after_max_crashes_in_window() {
        let g = RestartGovernor::default();
        let t0 = 1_000_000;
        // No history → unthrottled.
        assert_eq!(g.gate("c", t0), Ok(0));
        // Crashes within the window accumulate.
        assert_eq!(g.record_crash("c", t0), 1);
        assert_eq!(g.record_crash("c", t0 + 6_000), 2);
        assert_eq!(g.record_crash("c", t0 + 12_000), 3);
        // At the cap the breaker trips.
        assert_eq!(g.gate("c", t0 + 12_000), Err(RESTART_MAX_PER_WINDOW));
    }

    #[test]
    fn restart_governor_backoff_grows_per_crash() {
        let g = RestartGovernor::default();
        let t0 = 5_000;
        g.record_crash("c", t0); // count 1
        assert_eq!(g.gate("c", t0), Ok(1_000));
        g.record_crash("c", t0); // count 2
        assert_eq!(g.gate("c", t0), Ok(2_000));
    }

    #[test]
    fn restart_governor_resets_after_window() {
        let g = RestartGovernor::default();
        let t0 = 0;
        g.record_crash("c", t0);
        g.record_crash("c", t0 + 1_000);
        g.record_crash("c", t0 + 2_000); // count 3 → tripped
        assert!(g.gate("c", t0 + 2_000).is_err());
        // A crash after the window has elapsed starts a fresh budget.
        let after = t0 + RESTART_WINDOW_MS + 3_000;
        assert_eq!(g.record_crash("c", after), 1);
        assert_eq!(g.gate("c", after), Ok(1_000));
    }

    #[test]
    fn restart_governor_forget_clears_history() {
        let g = RestartGovernor::default();
        let t0 = 100;
        g.record_crash("c", t0);
        g.record_crash("c", t0);
        g.record_crash("c", t0); // tripped
        assert!(g.gate("c", t0).is_err());
        g.forget("c");
        assert_eq!(g.gate("c", t0), Ok(0)); // fresh
    }

    #[tokio::test]
    async fn agent_error_recovery_crashes_trip_the_restart_breaker() {
        let registry = make_registry();
        // A genuine crash loop: each crash-eviction is followed by a respawn
        // that crashes again. Every cycle kills a distinct runtime instance,
        // so every cycle consumes restart budget. At the cap, gate() returns
        // Err *before* any backoff sleep on the next build.
        for _ in 0..RESTART_MAX_PER_WINDOW {
            registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
            registry.terminate("c", Some(AgentKillReason::AgentErrorRecovery)).unwrap();
        }
        // The next respawn is refused — the loop is broken.
        match registry.get_or_create_runtime("c", make_runtime_options("c")).await {
            Err(AppError::Conflict(_)) => {}
            Err(other) => panic!("expected Conflict, got {other:?}"),
            Ok(_) => panic!("crash loop must trip the breaker, but the build succeeded"),
        }
    }

    #[tokio::test]
    async fn terminations_without_a_runtime_never_trip_the_breaker() {
        let registry = make_registry();
        // No runtime was ever built: these terminations tear down nothing, so
        // they must not count as crashes. (Administrative teardown requests —
        // summon/skill-snapshot recycles on an idle conversation — used to be
        // recorded here as phantom crashes and paused the conversation.)
        for _ in 0..(RESTART_MAX_PER_WINDOW + 4) {
            registry.terminate("c", Some(AgentKillReason::AgentErrorRecovery)).unwrap();
        }
        assert!(
            registry.get_or_create_runtime("c", make_runtime_options("c")).await.is_ok(),
            "terminating a conversation with no runtime must not consume restart budget"
        );
    }

    #[tokio::test]
    async fn repeated_teardowns_of_one_runtime_count_a_single_crash() {
        let registry = make_registry();
        registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
        // Retried/duplicate teardown requests for the SAME runtime instance
        // (e.g. terminate_runtime_until_confirmed's retry loop, or overlapping
        // recovery owners) are one crash, not many.
        for _ in 0..(RESTART_MAX_PER_WINDOW + 4) {
            registry.terminate("c", Some(AgentKillReason::AgentErrorRecovery)).unwrap();
        }
        assert!(
            registry.get_or_create_runtime("c", make_runtime_options("c")).await.is_ok(),
            "one crashed runtime instance must consume exactly one unit of restart budget"
        );
    }

    #[tokio::test]
    async fn configuration_recycles_never_trip_the_breaker() {
        let registry = make_registry();
        for _ in 0..6 {
            registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
            // Config-driven rebuild (model/skills/summon change) is a
            // deliberate recycle, not a crash.
            registry.terminate("c", Some(AgentKillReason::ConfigurationChanged)).unwrap();
        }
        assert!(registry.get_or_create_runtime("c", make_runtime_options("c")).await.is_ok());
    }

    #[tokio::test]
    async fn benign_recycles_never_trip_the_breaker() {
        let registry = make_registry();
        for _ in 0..6 {
            registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
            // Knowledge-binding rebuild is a deliberate recycle, not a crash.
            registry.terminate("c", Some(AgentKillReason::KnowledgeBindingChanged)).unwrap();
        }
        // Never counted as a crash → still builds.
        assert!(registry.get_or_create_runtime("c", make_runtime_options("c")).await.is_ok());
    }

    #[tokio::test]
    async fn conversation_delete_resets_the_restart_governor() {
        let registry = make_registry();
        for _ in 0..RESTART_MAX_PER_WINDOW {
            registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
            registry.terminate("c", Some(AgentKillReason::AgentErrorRecovery)).unwrap();
        }
        // Deleting the conversation clears the crash history so a reused id starts fresh.
        registry.terminate("c", Some(AgentKillReason::ConversationDeleted)).unwrap();
        assert!(registry.get_or_create_runtime("c", make_runtime_options("c")).await.is_ok());
    }
}
