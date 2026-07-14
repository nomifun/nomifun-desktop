//! Process-local registry for live Agent runtimes.
//!
//! A registry entry is keyed by conversation ID and owns the long-lived Agent
//! process/session reused across turns. It is not a persisted execution step or
//! DAG task. Turn admission is handled separately by Conversation's
//! `AgentTurnHandle`.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use dashmap::DashMap;
use futures_util::future::BoxFuture;
use nomi_agent::session::SessionManager;
use nomifun_common::{
    AgentKillReason, AgentType, AppError, ConversationStatus, ErrorChain, OnConversationDelete, TimestampMs, now_ms,
};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use tracing::{info, warn};

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

    /// Terminate and remove a runtime.
    fn terminate(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError>;

    /// Terminate a runtime and resolve after its process has exited.
    fn terminate_and_wait(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

    /// Terminate and remove every active runtime.
    fn terminate_all(&self);

    /// Number of fully initialized active runtimes.
    fn active_runtime_count(&self) -> usize;

    /// Collect runtimes eligible for idle cleanup.
    ///
    /// Returns conversation IDs of runtimes that:
    /// - have `status == Some(Finished)`
    /// - have been idle longer than `idle_threshold_ms`
    fn collect_idle_runtimes(&self, idle_threshold_ms: TimestampMs) -> Vec<String>;
}

/// Per-conversation slot: an [`OnceCell`] that the first concurrent caller
/// initialises by running the factory, and that every subsequent caller
/// awaits. Failed initialisations leave the cell empty so the next caller
/// may retry; the slot itself is only removed by explicit termination.
type RuntimeSlot = Arc<OnceCell<AgentRuntimeHandle>>;

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
const MAX_CACHED_LIFECYCLE_GATES: usize = 256;

#[derive(Clone, Copy)]
struct RestartRecord {
    /// Crash-evictions inside the current window.
    count: u32,
    /// When the current window began (`now_ms`).
    window_start_ms: i64,
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
    }

    fn clear(&self) {
        self.records.clear();
    }
}

/// Default implementation of [`AgentRuntimeRegistry`] using a concurrent hash map.
pub struct InMemoryAgentRuntimeRegistry {
    runtimes: Arc<DashMap<String, RuntimeSlot>>,
    /// Serializes build and awaitable teardown for each conversation. The gate
    /// intentionally outlives a removed runtime slot so no replacement factory
    /// can start while the old agent is still unwinding.
    lifecycle_gates: Arc<DashMap<String, Weak<AsyncMutex<()>>>>,
    factory: AgentRuntimeFactory,
    /// Bounds rapid crash-respawn loops per conversation (see [`RestartGovernor`]).
    governor: RestartGovernor,
}

impl InMemoryAgentRuntimeRegistry {
    pub fn new(factory: AgentRuntimeFactory) -> Self {
        Self {
            runtimes: Arc::new(DashMap::new()),
            lifecycle_gates: Arc::new(DashMap::new()),
            factory,
            governor: RestartGovernor::default(),
        }
    }

    /// Look up a fully initialized runtime by conversation ID.
    fn initialized_runtime(&self, conversation_id: &str) -> Option<AgentRuntimeHandle> {
        self.runtimes
            .get(conversation_id)
            .and_then(|slot| slot.get().cloned())
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

    /// Feed a termination into the restart governor. Only a crash-recovery eviction
    /// ([`AgentKillReason::AgentErrorRecovery`]) counts against the respawn
    /// budget; every other termination is a deliberate recycle and must not. A
    /// definitive teardown ([`AgentKillReason::ConversationDeleted`]) drops the
    /// bookkeeping so a reused conversation id starts fresh.
    fn note_termination_for_governor(&self, conversation_id: &str, reason: Option<AgentKillReason>) {
        match reason {
            Some(AgentKillReason::AgentErrorRecovery) => {
                let count = self.governor.record_crash(conversation_id, now_ms());
                warn!(
                    conversation_id,
                    crash_count = count,
                    "Recorded agent crash-eviction for the restart governor"
                );
            }
            Some(AgentKillReason::ConversationDeleted) => self.governor.forget(conversation_id),
            _ => {}
        }
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
        let lifecycle_gate = self.lifecycle_gate(conversation_id);
        let _lifecycle = lifecycle_gate.lock().await;

        // Atomically obtain the per-conversation slot. `DashMap::entry` is
        // synchronous and side-effect-free — only an empty OnceCell is
        // allocated on the miss path, so concurrent callers for the same id
        // all end up holding the same `Arc<OnceCell>`.
        let slot: RuntimeSlot = self
            .runtimes
            .entry(conversation_id.to_owned())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone();

        // Fast path: a live runtime already exists — hand it back without
        // touching the restart governor (a healthy warm runtime is never
        // throttled).
        if let Some(runtime) = slot.get() {
            return Ok(runtime.clone());
        }

        // About to (re)build. Enforce the crash-loop governor so a
        // deterministically-crashing conversation cannot hot-loop respawns.
        match self.governor.gate(conversation_id, now_ms()) {
            Ok(0) => {}
            Ok(backoff_ms) => {
                warn!(
                    conversation_id,
                    backoff_ms, "Backing off before respawning a recently-crashed agent"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            }
            Err(count) => {
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

        // `OnceCell::get_or_try_init` serialises concurrent initialisers:
        // the first caller to reach it runs the factory, every other caller
        // awaits the same future and ends up with the same runtime. On
        // failure the cell stays empty so a later caller can retry.
        let factory = self.factory.clone();
        let runtime = slot.get_or_try_init(|| async move { factory(options).await }).await?;

        // `terminate` is intentionally synchronous and therefore cannot await
        // this conversation's lifecycle gate. It may remove the slot while a
        // slow factory is still initializing. Re-check map identity after the
        // factory settles: a removed/replaced slot is a termination tombstone,
        // so the just-created process must be stopped instead of escaping the
        // registry as an untracked live runtime.
        let slot_is_current = self
            .runtimes
            .get(conversation_id)
            .is_some_and(|entry| Arc::ptr_eq(entry.value(), &slot));
        if !slot_is_current {
            let _ = runtime.kill(None);
            return Err(AppError::Conflict(format!(
                "Agent runtime for conversation {conversation_id} was terminated while initializing"
            )));
        }
        Ok(runtime.clone())
    }

    fn terminate(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.note_termination_for_governor(conversation_id, reason);
        if let Some((id, slot)) = self.runtimes.remove(conversation_id) {
            info!(conversation_id = %id, ?reason, "Terminating Agent runtime");
            if let Some(agent) = slot.get() {
                agent.kill(reason)?;
            }
        }
        Ok(())
    }

    fn terminate_and_wait(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        self.note_termination_for_governor(conversation_id, reason);
        let conversation_id = conversation_id.to_owned();
        let lifecycle_gate = self.lifecycle_gate(&conversation_id);
        let runtimes = Arc::clone(&self.runtimes);
        Box::pin(async move {
            let _lifecycle = lifecycle_gate.lock().await;
            if let Some((id, slot)) = runtimes.remove(&conversation_id) {
                info!(conversation_id = %id, ?reason, "Terminating Agent runtime (awaitable)");
                if let Some(agent) = slot.get() {
                    agent.kill_and_wait(reason).await;
                }
            }
        })
    }

    fn terminate_all(&self) {
        self.governor.clear();
        let keys: Vec<String> = self.runtimes.iter().map(|r| r.key().clone()).collect();
        for key in keys {
            if let Some((id, slot)) = self.runtimes.remove(&key) {
                info!(conversation_id = %id, "Terminating Agent runtime during registry shutdown");
                if let Some(agent) = slot.get() {
                    let _ = agent.kill(None);
                }
            }
        }
    }

    fn active_runtime_count(&self) -> usize {
        self.runtimes
            .iter()
            .filter(|entry| entry.value().get().is_some())
            .count()
    }

    fn collect_idle_runtimes(&self, idle_threshold_ms: TimestampMs) -> Vec<String> {
        let now = now_ms();
        self.runtimes
            .iter()
            .filter_map(|entry| {
                let agent = entry.value().get()?;
                // Only ACP agents participate in idle cleanup per API Spec
                (agent.agent_type() == AgentType::Acp
                    && agent.status() == Some(ConversationStatus::Finished)
                    && (now - agent.last_activity_at()) > idle_threshold_ms)
                    .then(|| entry.key().clone())
            })
            .collect()
    }
}

/// Wired up by `nomifun-app` so deleting a conversation tears down its
/// agent process. Without this hook, ACP/nomi/nanobot subprocesses keep
/// streaming events for a `conversation_id` whose DB row is already gone
/// (Sentry ELECTRON-1BD).
#[async_trait]
impl OnConversationDelete for InMemoryAgentRuntimeRegistry {
    async fn on_conversation_deleted(&self, _user_id: &str, conversation_id: i64) {
        // The registry keys live runtimes by the String conversation id;
        // bridge the i64 hook key back to that form for termination.
        let conversation_id = conversation_id.to_string();
        if let Err(e) = self.terminate(&conversation_id, Some(AgentKillReason::ConversationDeleted)) {
            warn!(
                conversation_id,
                error = %ErrorChain(&e),
                "Failed to terminate Agent runtime on conversation delete (non-fatal)",
            );
        }
    }
}

/// Conversation-delete hook that removes a conversation's on-disk nomi state:
/// the global `nomi-sessions/*_{id}.json` file (+ index entry) and any legacy
/// id-named temp workspace under `work_dir/conversations`.
///
/// Without this, those files outlive the conversation. The session dir is keyed
/// only by the reusable integer conversation id, so an orphan could later be
/// resumed by a brand-new conversation that reuses the id (e.g. after a DB
/// rebaseline) — the cross-conversation "memory bleed" this guards against,
/// complementing the per-session `owner_token` check in the nomi factory.
/// Best-effort: every failure is logged, never fatal.
pub struct NomiSessionFilesCascade {
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
}

#[async_trait]
impl OnConversationDelete for NomiSessionFilesCascade {
    async fn on_conversation_deleted(&self, _user_id: &str, conversation_id: i64) {
        let id = conversation_id.to_string();

        // 1) nomi session transcript file + index entry.
        let session_manager = SessionManager::new(self.data_dir.join("nomi-sessions"), 100);
        if let Err(e) = session_manager.delete_session(&id) {
            warn!(conversation_id, error = %e, "Failed to delete nomi session file on conversation delete (non-fatal)");
        }

        // 2) legacy auto-provisioned temp workspace(s) named `{label}-temp-{id}`.
        //    Exact suffix match is id-safe (`-temp-3` never matches `-temp-13`).
        //    New token-named managed workspaces are deleted by ConversationService
        //    while it still has the full conversation row.
        let conv_dir = self.work_dir.join("conversations");
        let suffix = format!("-temp-{id}");
        if let Ok(entries) = std::fs::read_dir(&conv_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().ends_with(&suffix) && entry.path().is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                        warn!(conversation_id, error = %e, "Failed to remove temp workspace on conversation delete (non-fatal)");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_handle::{AgentRuntimeControl, MockAgentRuntime};
    use crate::protocol::events::AgentStreamEvent;
    use crate::types::SendMessageData;
    use futures_util::FutureExt;
    use nomifun_common::{AgentKillReason, AgentType, ConversationStatus, ProviderWithModel};
    use std::sync::atomic::{AtomicI64, Ordering};
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
        kill_started: Option<Arc<Semaphore>>,
        kill_release: Option<Arc<Semaphore>>,
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
                kill_started: None,
                kill_release: None,
            }
        }

        fn with_blocking_kill(mut self, started: Arc<Semaphore>, release: Arc<Semaphore>) -> Self {
            self.kill_started = Some(started);
            self.kill_release = Some(release);
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
        fn last_activity_at(&self) -> TimestampMs {
            self.last_activity.load(Ordering::Relaxed)
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
        fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
            Ok(())
        }
    }

    impl MockAgentRuntime for MockAgent {
        fn kill_and_wait(
            &self,
            reason: Option<AgentKillReason>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
            let _ = self.kill(reason);
            let started = self.kill_started.clone();
            let release = self.kill_release.clone();
            Box::pin(async move {
                if let Some(started) = started {
                    started.add_permits(1);
                }
                if let Some(release) = release {
                    let _ = release.acquire().await;
                }
            })
        }
    }

    fn make_runtime_options(conversation_id: &str) -> AgentRuntimeBuildOptions {
        AgentRuntimeBuildOptions {
            user_id: "test-user".into(),
            agent_type: AgentType::Acp,
            workspace: "/tmp/test".into(),
            model: ProviderWithModel {
                provider_id: "p1".into(),
                model: "test".into(),
                use_model: None,
            },
            conversation_id: conversation_id.into(),
            delegation_policy: Default::default(),
            extra: serde_json::Value::Null,
            conversation_created_at: None,
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
        // Second call retries and succeeds.
        let h = registry.get_or_create_runtime("conv-1", make_runtime_options("conv-1")).await.unwrap();
        assert_eq!(h.conversation_id(), "conv-1");
        assert_eq!(registry.active_runtime_count(), 1);
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
                registry.terminate_and_wait("conv-teardown", None).await;
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
        // Initial build succeeds.
        registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
        // Crash-evictions accumulate (recorded whether or not a rebuild
        // intervenes). At the cap, gate() returns Err *before* any backoff
        // sleep, so the test needs no clock control.
        for _ in 0..RESTART_MAX_PER_WINDOW {
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
        registry.get_or_create_runtime("c", make_runtime_options("c")).await.unwrap();
        for _ in 0..RESTART_MAX_PER_WINDOW {
            registry.terminate("c", Some(AgentKillReason::AgentErrorRecovery)).unwrap();
        }
        // Deleting the conversation clears the crash history so a reused id starts fresh.
        registry.terminate("c", Some(AgentKillReason::ConversationDeleted)).unwrap();
        assert!(registry.get_or_create_runtime("c", make_runtime_options("c")).await.is_ok());
    }
}
