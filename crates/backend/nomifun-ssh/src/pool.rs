//! `SshConnectionPool`: one live SSH link per (conversation, host), supervised.
//!
//! The pool exists because an SSH session is not a request. The model expects the
//! cwd it left behind, the shell environment it exported and the file handles it
//! opened to still be there on its next turn — and the agent runtime that asked
//! for the connection is destroyed and rebuilt whenever the operator switches
//! models. So the link outlives the runtime, is keyed by conversation, and is
//! owned here.
//!
//! # The one invariant
//!
//! Each link owns a `watch` of [`SshLinkState`], and [`PoolInner::publish`] is its
//! only writer. That same call projects the new value onto the wire and hands it
//! to the emitter, so "what the socket is doing", "what the REST snapshot says"
//! and "what the operator's browser was told" cannot drift apart. Everything else
//! in this file reads that value; nothing else writes it.
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use dashmap::DashMap;
use nomifun_ai_agent::{RemoteCommandOutput, SshBackend};
use nomifun_common::SshHostId;
use tokio::sync::{watch, Notify, RwLock};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::dto::SshStatusEvent;
use crate::events::SshEventEmitter;
use crate::service::SshHostService;
use crate::sink::{
    SshActionDispatchError, SshConnectionHandle, SshDialError, SshLinkBackend,
};
use crate::state::{
    SshLinkState, SshTeardown, SSH_CLOSE_BUDGET, SSH_DIAL_TIMEOUT, SSH_LIVENESS_POLL_INTERVAL,
    SSH_RECONNECT_INITIAL_BACKOFF_MS, SSH_RECONNECT_MAX_ATTEMPTS, SSH_RECONNECT_MAX_BACKOFF_MS,
};

/// How long one failure closes a host's dial gate to everybody else. Matched to
/// the ladder's first rung so a single link never trips its own cooldown, while a
/// crowd of links against one host collapses to roughly one dial per second.
pub const SSH_DIAL_COOLDOWN: Duration =
    Duration::from_millis(SSH_RECONNECT_INITIAL_BACKOFF_MS);
/// A terminal failure (rejected credential, changed host key) is held for longer
/// than a transient one: it will keep failing until a human changes something, so
/// every waiter in the burst should get the answer without its own TCP attempt.
const TERMINAL_COOLDOWN_FACTOR: u32 = 10;
/// How long a liveness round trip may take before the link counts as half-open.
/// Below the poll interval so probes cannot pile up on each other.
pub const SSH_PING_TIMEOUT: Duration = Duration::from_secs(10);
/// Budget for the one command a test-connection probe runs.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Timing knobs for the supervisor. Production uses [`PoolTuning::default`],
/// which is exactly the ladder pinned in [`crate::state`]; tests shrink it so a
/// reconnect case does not have to sit through the real 1s→60s ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolTuning {
    pub liveness_poll: Duration,
    pub ping_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub max_attempts: u32,
    pub dial_cooldown: Duration,
    pub close_budget: Duration,
}

impl Default for PoolTuning {
    fn default() -> Self {
        Self {
            liveness_poll: SSH_LIVENESS_POLL_INTERVAL,
            ping_timeout: SSH_PING_TIMEOUT,
            initial_backoff: Duration::from_millis(SSH_RECONNECT_INITIAL_BACKOFF_MS),
            max_backoff: Duration::from_millis(SSH_RECONNECT_MAX_BACKOFF_MS),
            max_attempts: SSH_RECONNECT_MAX_ATTEMPTS,
            dial_cooldown: SSH_DIAL_COOLDOWN,
            close_budget: SSH_CLOSE_BUDGET,
        }
    }
}

impl PoolTuning {
    /// How long before retry number `attempt` (1-based). The default tuning walks
    /// the 1s→60s ladder the constants in [`crate::state`] describe, pinned
    /// literally by this module's tests.
    pub fn delay(&self, attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1);
        match self.initial_backoff.checked_mul(1u32 << doublings.min(31)) {
            Some(d) => d.min(self.max_backoff),
            None => self.max_backoff,
        }
    }
}

/// Identifies one pooled link. The host is part of the key, so rebinding a
/// conversation to a different host cannot reuse the old socket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SshLinkKey {
    pub conversation_id: String,
    pub ssh_host_id: SshHostId,
}

impl SshLinkKey {
    pub fn new(conversation_id: impl Into<String>, ssh_host_id: SshHostId) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            ssh_host_id,
        }
    }
}

/// One pooled link. The `handle` is swapped underneath on reconnect, so an
/// `Arc<dyn SshBackend>` the agent is already holding keeps working across an
/// outage instead of having to be rebuilt (which it cannot be — the runtime that
/// asked for it may be gone).
pub struct SshLink {
    /// Distinguishes successive physical links that reuse the same
    /// `(AgentSession, ssh_host)` key. A teardown loss belongs to one exact
    /// incarnation and must survive a later successful reconnect/close.
    instance_id: u64,
    key: SshLinkKey,
    owner_id: String,
    handle: RwLock<Option<Arc<SshConnectionHandle>>>,
    /// The last cwd the shell's sentinel actually proved. Replayed as the remote
    /// cwd on redial, because a reconnect that silently drops the model back into
    /// `$HOME` is worse than a visible failure.
    last_cwd: std::sync::Mutex<String>,
    state_tx: watch::Sender<SshLinkState>,
    /// When the link last actually changed state. Kept beside the `watch` because
    /// the value the client orders by has to survive being re-read: a snapshot
    /// that stamped itself with the current time would report when it was asked,
    /// not when anything happened.
    changed_at: AtomicI64,
    /// Serializes dial / recycle / close for this link, so two of them cannot
    /// both decide what the `handle` slot should contain.
    transition: tokio::sync::Mutex<()>,
    /// Nudges the supervisor when a tool call notices the link died, so the ladder
    /// starts now rather than at the next liveness tick.
    wake: Notify,
    /// Per-link action admission. Closing a link retires this gate before it
    /// touches the transport, then waits for every action that linearized first.
    /// An old backend may keep the `Arc<SshLink>` forever, but it can never use a
    /// replacement transport after this gate is retired.
    actions: Arc<ActionAdmission>,
    /// One process-owned teardown and its immutable terminal receipt. Closing
    /// callers may be cancelled independently; they never own the close task.
    close: Arc<LinkCloseCompletion>,
}

impl SshLink {
    fn new(instance_id: u64, key: SshLinkKey, owner_id: &str, remote_cwd: &str) -> Self {
        let (state_tx, _) = watch::channel(SshLinkState::Idle);
        Self {
            instance_id,
            key,
            owner_id: owner_id.to_string(),
            handle: RwLock::new(None),
            last_cwd: std::sync::Mutex::new(remote_cwd.to_string()),
            state_tx,
            changed_at: AtomicI64::new(nomifun_common::now_ms()),
            transition: tokio::sync::Mutex::new(()),
            wake: Notify::new(),
            actions: Arc::new(ActionAdmission::default()),
            close: Arc::new(LinkCloseCompletion::default()),
        }
    }

    pub fn key(&self) -> &SshLinkKey {
        &self.key
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn state(&self) -> SshLinkState {
        self.state_tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<SshLinkState> {
        self.state_tx.subscribe()
    }

    /// When this link last changed state.
    pub fn changed_at(&self) -> nomifun_common::TimestampMs {
        self.changed_at.load(Ordering::Relaxed)
    }

    /// The last cwd proven by a command sentinel (or the cwd the session was
    /// created with, before any command has run).
    pub fn last_cwd(&self) -> String {
        self.last_cwd.lock().unwrap().clone()
    }

    pub(crate) fn remember_cwd(&self, cwd: &str) {
        *self.last_cwd.lock().unwrap() = cwd.to_string();
    }

    pub(crate) async fn current_handle(&self) -> Option<Arc<SshConnectionHandle>> {
        self.handle.read().await.clone()
    }

    async fn has_live_transport(&self) -> bool {
        self.handle
            .read()
            .await
            .as_ref()
            .is_some_and(|h| !h.is_transport_closed())
    }
}

/// Shared terminal state for an exact physical link close. `started` is claimed
/// synchronously before the process-owned task is spawned, so dropping any
/// request future cannot abandon teardown or let a second caller invent a
/// different result.
#[derive(Default)]
struct LinkCloseCompletion {
    started: AtomicBool,
    receipt: OnceLock<SshTeardown>,
    finished: Notify,
}

impl LinkCloseCompletion {
    fn try_start(&self) -> bool {
        self.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn finish(&self, teardown: SshTeardown) {
        if self.receipt.set(teardown).is_ok() {
            self.finished.notify_waiters();
        }
    }

    async fn wait(&self) -> SshTeardown {
        loop {
            // Tokio's Notified snapshots the notify_waiters generation at
            // construction, so a finish between this line and the receipt read
            // is still observed by the subsequent await.
            let finished = self.finished.notified();
            if let Some(teardown) = self.receipt.get() {
                return teardown.clone();
            }
            finished.await;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TeardownReceiptKey {
    link: SshLinkKey,
    instance_id: u64,
}

#[derive(Default)]
struct TeardownReceipts {
    entries: DashMap<TeardownReceiptKey, SshTeardown>,
}

impl TeardownReceipts {
    fn record(&self, key: TeardownReceiptKey, teardown: SshTeardown) {
        self.entries.insert(key, teardown);
    }

    fn latest_for_link(
        &self,
        key: &SshLinkKey,
    ) -> Option<(TeardownReceiptKey, SshTeardown)> {
        self.entries
            .iter()
            .filter(|entry| &entry.key().link == key)
            .max_by_key(|entry| entry.key().instance_id)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
    }

    fn for_session(&self, agent_session_id: &str) -> Vec<(TeardownReceiptKey, SshTeardown)> {
        self.entries
            .iter()
            .filter(|entry| entry.key().link.conversation_id == agent_session_id)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    fn acknowledge_persisted_for_session(&self, agent_session_id: &str) -> usize {
        let keys = self
            .entries
            .iter()
            .filter(|entry| entry.key().link.conversation_id == agent_session_id)
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter(|key| self.entries.remove(key).is_some())
            .count()
    }

    fn discard_proven(&self, key: &TeardownReceiptKey) {
        if self
            .entries
            .get(key)
            .is_some_and(|entry| !matches!(entry.value(), SshTeardown::Lost { .. }))
        {
            self.entries.remove(key);
        }
    }
}

#[derive(Default)]
struct ActionAdmissionState {
    retired: bool,
    in_flight: usize,
}

/// A tiny synchronous admission gate. The mutex is held only while changing two
/// integers; no network or async work occurs under it. `retire` and `admit` use
/// the same critical section, which is the linearization point needed by Session
/// deletion: either the action owns a lease and cleanup waits, or retirement won
/// and the action never reaches a handle.
#[derive(Default)]
struct ActionAdmission {
    state: Mutex<ActionAdmissionState>,
    drained: Notify,
}

impl ActionAdmission {
    fn admit(self: &Arc<Self>) -> Option<ActionAdmissionLease> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.retired {
            return None;
        }
        state.in_flight += 1;
        Some(ActionAdmissionLease {
            admission: Arc::clone(self),
        })
    }

    fn retire(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.retired = true;
        if state.in_flight == 0 {
            self.drained.notify_waiters();
        }
    }

    fn is_retired(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retired
    }

    async fn wait_drained(&self) {
        loop {
            // Register before inspecting the count so the final lease cannot
            // notify between our check and the await.
            let drained = self.drained.notified();
            if self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .in_flight
                == 0
            {
                return;
            }
            drained.await;
        }
    }
}

struct ActionAdmissionLease {
    admission: Arc<ActionAdmission>,
}

impl Drop for ActionAdmissionLease {
    fn drop(&mut self) {
        let mut state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        debug_assert!(state.in_flight > 0, "SSH action admission underflow");
        state.in_flight = state.in_flight.saturating_sub(1);
        if state.in_flight == 0 {
            self.admission.drained.notify_waiters();
        }
    }
}

/// Held from before the first handle lookup until the action has a terminal
/// result. Both fields matter: the Session lease makes deletion wait across all
/// host links, while the link lease makes ordinary link/host shutdown wait.
pub(crate) struct SshActionLease {
    _global: ActionAdmissionLease,
    _session: ActionAdmissionLease,
    _link: ActionAdmissionLease,
}

impl std::fmt::Debug for SshLink {
    /// Deliberately shallow: the handle behind it owns a credential-bearing
    /// transport, and a link is only ever printed to explain a lifecycle failure.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshLink")
            .field("conversation_id", &self.key.conversation_id)
            .field("ssh_host_id", &self.key.ssh_host_id)
            .field("phase", &self.state().phase())
            .finish_non_exhaustive()
    }
}

/// What a test-connection probe found. A probe never joins the pool: it dials,
/// runs one trivial command and closes, so clicking "test" cannot silently graft
/// a link onto a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshProbeOutcome {
    pub ok: bool,
    pub host_fingerprint: Option<String>,
    pub detail: String,
}

/// What `shutdown_all` managed to prove about the links it let go of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SshShutdownReport {
    /// Remote shell confirmed dead (exit status or signal).
    pub reaped: usize,
    /// Let go of without proof — a teardown *failure*, not a quieter success.
    pub lost: usize,
    /// Already gone when we got there; nothing to reap and nothing wrong.
    pub already_down: usize,
}

impl SshShutdownReport {
    pub fn total(&self) -> usize {
        self.reaped + self.lost + self.already_down
    }
}

/// Whether a dial was asked for by a person or by the supervisor. Only a person
/// may dial a host that was withdrawn from the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialIntent {
    /// `acquire` / `probe`: the operator is asking, so a stale retirement is
    /// forgotten (they have most likely just fixed whatever was wrong).
    Fresh,
    /// The reconnect ladder. Never revives a withdrawn host.
    Redial,
}

/// Why a host's dial gate is shut.
enum GateBlock {
    /// A failure recent enough that another TCP attempt would learn nothing new.
    /// Honoured by everyone: ten conversations against one rebooting host produce
    /// one dial per cooldown, not ten simultaneous ones.
    Cooling { until: Instant, err: SshDialError },
    /// The host was withdrawn (deleted, or its credential changed under us). A
    /// supervisor must never dial it again.
    Retired { err: SshDialError },
}

/// Per-host dial gate: one dial in flight per **host** (not per link), plus a
/// shared memory of the last failure.
struct HostGate {
    dialing: tokio::sync::Mutex<()>,
    block: std::sync::Mutex<Option<GateBlock>>,
}

impl HostGate {
    fn new() -> Self {
        Self {
            dialing: tokio::sync::Mutex::new(()),
            block: std::sync::Mutex::new(None),
        }
    }

    /// The answer a dial should get without touching the network, if any. Expired
    /// cooldowns and (for a deliberate dial) retirements are cleared in passing.
    fn blocked(&self, intent: DialIntent) -> Option<SshDialError> {
        let mut block = self.block.lock().unwrap();
        match &*block {
            Some(GateBlock::Cooling { until, err }) => {
                if Instant::now() < *until {
                    Some(err.clone())
                } else {
                    *block = None;
                    None
                }
            }
            Some(GateBlock::Retired { err }) => match intent {
                DialIntent::Fresh => {
                    *block = None;
                    None
                }
                DialIntent::Redial => Some(err.clone()),
            },
            None => None,
        }
    }

    fn record_failure(&self, err: SshDialError, cooldown: Duration) {
        let window = if err.is_retryable() {
            cooldown
        } else {
            cooldown * TERMINAL_COOLDOWN_FACTOR
        };
        *self.block.lock().unwrap() = Some(GateBlock::Cooling {
            until: Instant::now() + window,
            err,
        });
    }

    fn retire(&self, err: SshDialError) {
        *self.block.lock().unwrap() = Some(GateBlock::Retired { err });
    }

    fn clear(&self) {
        *self.block.lock().unwrap() = None;
    }
}

struct PoolInner {
    service: SshHostService,
    known_hosts: PathBuf,
    events: SshEventEmitter,
    tuning: PoolTuning,
    links: DashMap<SshLinkKey, Arc<SshLink>>,
    next_link_instance: AtomicU64,
    /// Process-owned terminal evidence for exact link incarnations. Proven
    /// receipts may be discarded after a successful Session sweep; Lost stays
    /// until the canonical delete saga confirms its durable Store uncertainty.
    teardown_receipts: TeardownReceipts,
    /// Process-wide admission used only to make `shutdown_all` a true fence.
    global_admission: Arc<ActionAdmission>,
    /// Permanent per-AgentSession fences. Session UUIDs are never reused, so a
    /// retired entry intentionally lives for the process lifetime.
    session_admissions: DashMap<String, Arc<ActionAdmission>>,
    supervisors: DashMap<SshLinkKey, JoinHandle<()>>,
    gates: DashMap<SshHostId, Arc<HostGate>>,
    /// Set by `shutdown_all`. A pool that is closing must not open a socket it
    /// will then fail to account for.
    quiescing: AtomicBool,
}

/// Process-level pool of live SSH links. `clone()` is a handle to the same pool,
/// never a copy of it — the whole point is that the router, the agent factory and
/// the conversation-delete hook all see the same sockets.
#[derive(Clone)]
pub struct SshConnectionPool(Arc<PoolInner>);

impl SshConnectionPool {
    pub fn new(service: SshHostService, known_hosts: PathBuf, events: SshEventEmitter) -> Self {
        Self::with_tuning(service, known_hosts, events, PoolTuning::default())
    }

    /// Same as [`SshConnectionPool::new`] with explicit timings. Public because
    /// the lifecycle tests live in a separate crate and would otherwise have to
    /// sit through the production ladder.
    pub fn with_tuning(
        service: SshHostService,
        known_hosts: PathBuf,
        events: SshEventEmitter,
        tuning: PoolTuning,
    ) -> Self {
        Self(Arc::new(PoolInner {
            service,
            known_hosts,
            events,
            tuning,
            links: DashMap::new(),
            next_link_instance: AtomicU64::new(1),
            teardown_receipts: TeardownReceipts::default(),
            global_admission: Arc::new(ActionAdmission::default()),
            session_admissions: DashMap::new(),
            supervisors: DashMap::new(),
            gates: DashMap::new(),
            quiescing: AtomicBool::new(false),
        }))
    }

    /// The host book this pool dials through — the same instance, so a credential
    /// edited via the routes is the credential the next redial uses.
    pub fn host_service(&self) -> SshHostService {
        self.0.service.clone()
    }

    /// Whether two handles are the same pool — same links, same sockets. A pool is
    /// an identity, not a value, so this is deliberately not `PartialEq`.
    pub fn is_same_pool(&self, other: &SshConnectionPool) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Whether this pool still holds the link for `key`. False once it has been
    /// closed (or was never opened), which is how a released lease tells "the pool
    /// kept my session" from "my session is gone".
    pub fn is_pooled(&self, key: &SshLinkKey) -> bool {
        self.0.links.contains_key(key)
    }

    /// The link for `(conversation_id, ssh_host_id)`, connected. Reuses the
    /// existing link when there is one; `remote_cwd` only seeds a brand-new link,
    /// because a live session's cwd belongs to the session, not the caller.
    pub async fn acquire(
        &self,
        user_id: &str,
        conversation_id: &str,
        ssh_host_id: &SshHostId,
        remote_cwd: &str,
    ) -> Result<Arc<SshLink>, SshDialError> {
        if self.0.quiescing.load(Ordering::SeqCst) {
            return Err(SshDialError::ShuttingDown);
        }
        let _global_lease = self
            .0
            .global_admission
            .admit()
            .ok_or(SshDialError::ShuttingDown)?;
        let session_admission = self.0.session_admission_for(conversation_id);
        let _session_lease = session_admission
            .admit()
            .ok_or(SshDialError::SessionRetired)?;
        let key = SshLinkKey::new(conversation_id, ssh_host_id.clone());
        let link = self.0.link_for(&key, user_id, remote_cwd);
        let _link_lease = link
            .actions
            .admit()
            .ok_or(SshDialError::LinkRetired)?;
        self.0.ensure_connected(&link).await?;
        Ok(link)
    }

    /// Permanently fence an exact AgentSession before its delete cleanup begins.
    /// This method is synchronous on purpose: a deletion owner can establish the
    /// no-new-SSH-work boundary before awaiting another resource owner. The later
    /// [`Self::close_conversation`] call is idempotent and performs the drain.
    pub fn retire_agent_session(&self, agent_session_id: &str) {
        self.0
            .session_admission_for(agent_session_id)
            .retire();
    }

    /// Admit one action against the exact Session and link. The returned lease
    /// must remain alive through the terminal transport result.
    pub(crate) fn action_lease(
        &self,
        link: &Arc<SshLink>,
    ) -> Result<SshActionLease, String> {
        if self.0.quiescing.load(Ordering::SeqCst) {
            return Err("the ssh connection pool is shutting down".to_string());
        }
        let global = self
            .0
            .global_admission
            .admit()
            .ok_or_else(|| "the ssh connection pool is shutting down".to_string())?;
        let session = self
            .0
            .session_admission_for(&link.key.conversation_id)
            .admit()
            .ok_or_else(|| {
                "the AgentSession is retired and cannot admit SSH actions".to_string()
            })?;
        let link_lease = link.actions.admit().ok_or_else(|| {
            "the SSH link is retired and cannot admit actions".to_string()
        })?;
        let is_current = self
            .0
            .links
            .get(link.key())
            .is_some_and(|current| Arc::ptr_eq(current.value(), link));
        if !is_current {
            return Err("the SSH link is no longer pooled for this AgentSession".to_string());
        }
        Ok(SshActionLease {
            _global: global,
            _session: session,
            _link: link_lease,
        })
    }

    /// The `SshBackend` for a link. Resolves the link's current handle per call,
    /// so handing this to the agent once survives any number of reconnects.
    pub fn backend_for(&self, link: &Arc<SshLink>) -> Arc<dyn SshBackend> {
        Arc::new(SshLinkBackend::new(self.clone(), Arc::clone(link)))
    }

    /// Execute one explicitly authorized elevated action on an isolated shell.
    /// The persistent shell used by ordinary `ssh/exec` never receives sudo
    /// responder rules.
    pub(crate) async fn run_sudo_action(
        &self,
        user_id: &str,
        link: &Arc<SshLink>,
        command: &str,
        timeout_ms: u64,
    ) -> Result<RemoteCommandOutput, SshActionDispatchError> {
        let action_lease = self
            .action_lease(link)
            .map_err(SshActionDispatchError::Rejected)?;
        if link.owner_id != user_id {
            return Err(SshActionDispatchError::Rejected(
                "ssh_host resource is not owned by this principal".into(),
            ));
        }
        let handle = link.current_handle().await.ok_or_else(|| {
            SshActionDispatchError::Rejected(format!(
                "ssh link for this session is not connected ({:?})",
                link.state().phase()
            ))
        })?;
        let credential = self
            .0
            .service
            .decrypt_credential(user_id, &link.key.ssh_host_id)
            .await
            .map_err(|error| SshActionDispatchError::Rejected(error.to_string()))?;
        let result = handle
            .run_ephemeral_sudo(
                &link.last_cwd(),
                command,
                timeout_ms,
                &credential,
            )
            .await;
        drop(action_lease);
        result
    }

    /// Execute an ordinary shell Action under the kernel's no-new-privileges
    /// bit. Hosts without `setpriv`, and root login sessions, fail closed so
    /// `ssh/exec` cannot bypass the separately granted `ssh/sudo` Action.
    pub(crate) async fn run_unprivileged_action(
        &self,
        user_id: &str,
        link: &Arc<SshLink>,
        command: &str,
        timeout_ms: u64,
    ) -> Result<RemoteCommandOutput, SshActionDispatchError> {
        let action_lease = self
            .action_lease(link)
            .map_err(SshActionDispatchError::Rejected)?;
        if link.owner_id != user_id {
            return Err(SshActionDispatchError::Rejected(
                "ssh_host resource is not owned by this principal".into(),
            ));
        }
        let handle = link.current_handle().await.ok_or_else(|| {
            SshActionDispatchError::Rejected(format!(
                "ssh link for this session is not connected ({:?})",
                link.state().phase()
            ))
        })?;
        let result = handle.run_unprivileged(command, timeout_ms).await;
        drop(action_lease);
        result
    }

    /// Watch one link's state. `None` when the pool has no such link.
    pub fn subscribe(&self, key: &SshLinkKey) -> Option<watch::Receiver<SshLinkState>> {
        self.0.links.get(key).map(|link| link.subscribe())
    }

    /// Every link owned by `user_id`, projected onto the same wire shape the
    /// realtime event uses — so a client that missed an event and re-fetches
    /// cannot see a different story.
    pub fn snapshot(&self, user_id: &str) -> Vec<SshStatusEvent> {
        self.0
            .links
            .iter()
            .filter(|entry| entry.owner_id == user_id)
            .map(|entry| {
                SshStatusEvent::from_state(
                    entry.key.ssh_host_id.as_str(),
                    &entry.key.conversation_id,
                    &entry.state(),
                    entry.changed_at(),
                )
            })
            .collect()
    }

    pub fn active_link_count(&self) -> usize {
        self.0.links.len()
    }

    /// Confirm that every currently retained SSH teardown receipt for this
    /// exact AgentSession has been durably reduced by the canonical Store.
    /// Proven receipts are retained too: if the Store write fails after the
    /// physical close, a same-process retry must observe the same proof rather
    /// than infer success from an empty pool.
    pub fn acknowledge_persisted_agent_session_teardowns(
        &self,
        agent_session_id: &str,
    ) -> usize {
        self.0
            .acknowledge_persisted_teardowns(agent_session_id)
    }

    /// Close one link and report what could be proven about it.
    pub async fn close_link(&self, key: &SshLinkKey) -> SshTeardown {
        let Some(link) = self.0.links.get(key).map(|entry| Arc::clone(entry.value())) else {
            if let Some((receipt_key, teardown)) = self.0.latest_teardown_receipt(key) {
                self.0.teardown_receipts.discard_proven(&receipt_key);
                return teardown;
            }
            return SshTeardown::AlreadyDown {
                    detail: "no pooled link for this session".to_string(),
                };
        };
        let receipt_key = TeardownReceiptKey {
            link: link.key.clone(),
            instance_id: link.instance_id,
        };
        let teardown = self.close_exact_link(link).await;
        self.0.teardown_receipts.discard_proven(&receipt_key);
        teardown
    }

    async fn close_exact_link(&self, link: Arc<SshLink>) -> SshTeardown {
        // Retire while the exact map entry is still published. An action that
        // acquired its lease first is allowed to finish and cleanup waits; an
        // action arriving now is rejected before it can read a handle.
        link.actions.retire();
        self.0.start_link_teardown(Arc::clone(&link));
        link.close.wait().await
    }

    /// Close every link bound to a conversation (a session may have been rebound
    /// and still hold a link to its previous host).
    pub async fn close_conversation(&self, conversation_id: &str) -> Vec<SshTeardown> {
        let session_admission = self.0.session_admission_for(conversation_id);
        // This is the permanent AgentSession fence. It must precede both the
        // drain and the key sweep so an acquire cannot publish a new host link
        // after the sweep took its snapshot.
        session_admission.retire();
        session_admission.wait_drained().await;
        let links = self
            .0
            .links_where(|key| key.conversation_id == conversation_id);
        let mut closed_instances = HashSet::with_capacity(links.len());
        let mut teardowns = Vec::with_capacity(links.len());
        for link in links {
            closed_instances.insert(link.instance_id);
            teardowns.push(self.close_exact_link(link).await);
        }
        // A previous caller may have been cancelled after teardown returned Lost
        // but before the App durably recorded the uncertainty. An empty active
        // link sweep must replay that loss, including losses from older physical
        // incarnations that reused the same `(Session, host)` key.
        for (receipt_key, teardown) in self.0.teardown_receipts_for_session(conversation_id) {
            if !closed_instances.contains(&receipt_key.instance_id) {
                teardowns.push(teardown);
            }
        }
        // This function has no await after this point. Every terminal receipt,
        // including Reaped/AlreadyDown, survives until the canonical Store
        // write is acknowledged; otherwise a cancelled/failed Store append
        // would turn the next retry's empty pool into invented success.
        teardowns
    }

    /// Withdraw a host: close its links and forbid supervisors from dialling it
    /// again. Called when a host is deleted or its credential is edited, so a
    /// supervisor cannot keep knocking on a door the operator just removed — or
    /// keep replaying a credential that no longer exists until the account locks.
    pub async fn close_for_host(&self, ssh_host_id: &SshHostId) {
        // Retire before closing: a supervisor woken by the teardown must find the
        // gate already shut.
        self.0
            .gate_for(ssh_host_id)
            .retire(SshDialError::Credential(
                "this ssh host was withdrawn from the pool".to_string(),
            ));
        for key in self.0.keys_where(|key| &key.ssh_host_id == ssh_host_id) {
            self.close_link(&key).await;
        }
    }

    /// Quiesce: refuse new links, then close everything that is open.
    pub async fn shutdown_all(&self) -> SshShutdownReport {
        // Refusing first is what makes the count honest — a link opened while we
        // were closing would be missed by the sweep and leak into shutdown.
        self.0.quiescing.store(true, Ordering::SeqCst);
        self.0.global_admission.retire();
        self.0.global_admission.wait_drained().await;
        let mut report = SshShutdownReport::default();
        for key in self.0.keys_where(|_| true) {
            match self.close_link(&key).await {
                SshTeardown::Reaped { .. } => report.reaped += 1,
                SshTeardown::Lost { detail } => {
                    warn!(
                        conversation_id = %key.conversation_id,
                        ssh_host_id = %key.ssh_host_id,
                        detail = %detail,
                        "ssh link let go of without proof the remote shell died"
                    );
                    report.lost += 1;
                }
                SshTeardown::AlreadyDown { .. } => report.already_down += 1,
            }
        }
        report
    }

    /// Test-connection probe: dial, run one trivial command, close. Nothing is
    /// pooled, so clicking "test" on the host book cannot leave a link behind or
    /// disturb a session already talking to that host.
    pub async fn probe(&self, user_id: &str, ssh_host_id: &SshHostId) -> SshProbeOutcome {
        if self.0.quiescing.load(Ordering::SeqCst) {
            return SshProbeOutcome {
                ok: false,
                host_fingerprint: None,
                detail: SshDialError::ShuttingDown.to_string(),
            };
        }
        let Some(_global_lease) = self.0.global_admission.admit() else {
            return SshProbeOutcome {
                ok: false,
                host_fingerprint: None,
                detail: SshDialError::ShuttingDown.to_string(),
            };
        };
        // The remote `$HOME` — a probe has no session and therefore no cwd to
        // honour.
        let handle = match self
            .0
            .dial_host(user_id, ssh_host_id, ".", DialIntent::Fresh)
            .await
        {
            Ok(handle) => handle,
            Err(e) => {
                let detail = e.to_string();
                let _ = self
                    .0
                    .service
                    .mark_unreachable(user_id, ssh_host_id, &detail)
                    .await;
                return SshProbeOutcome {
                    ok: false,
                    host_fingerprint: None,
                    detail,
                };
            }
        };

        let fingerprint = handle.fingerprint.clone();
        let probe = handle.shell().run("true", PROBE_TIMEOUT).await;
        let outcome = match probe {
            Ok(_) => {
                let _ = self
                    .0
                    .service
                    .mark_connected(user_id, ssh_host_id, fingerprint.as_deref())
                    .await;
                SshProbeOutcome {
                    ok: true,
                    host_fingerprint: fingerprint,
                    detail: "connection succeeded".to_string(),
                }
            }
            Err(e) => {
                let detail = format!("connected but the probe command failed: {e}");
                let _ = self
                    .0
                    .service
                    .mark_unreachable(user_id, ssh_host_id, &detail)
                    .await;
                SshProbeOutcome {
                    ok: false,
                    host_fingerprint: fingerprint,
                    detail,
                }
            }
        };
        handle.shell().close(self.0.tuning.close_budget).await;
        let _ = handle.conn().disconnect().await;
        outcome
    }

    /// Reopen a wedged shell on the same transport (see
    /// [`PoolInner::recycle_shell`]). Called by the link-backed backend.
    pub(crate) async fn recycle_shell(&self, link: &Arc<SshLink>, detail: &str) {
        self.0.recycle_shell(link, detail).await;
    }

    /// Dial a link the reconnect ladder gave up on, because a tool call asked for
    /// it. Treated as a deliberate dial, and still subject to the host's dial gate
    /// — so a model retrying in a loop cannot outpace the cooldown.
    pub(crate) async fn revive(&self, link: &Arc<SshLink>) -> Result<(), SshDialError> {
        self.0.ensure_connected(link).await
    }

    /// Report that a tool call found the transport gone.
    pub(crate) async fn note_transport_loss(&self, link: &Arc<SshLink>, detail: &str) {
        self.0.note_transport_loss(link, detail).await;
    }
}

/// The pool *is* the agent's SSH provider. There is deliberately no second
/// un-pooled path: a session that dialled on the side would be invisible to the
/// status routes, to the delete cascade and to shutdown accounting.
#[async_trait::async_trait]
impl nomifun_ai_agent::SshBackendProvider for SshConnectionPool {
    async fn connect(
        &self,
        user_id: &str,
        conversation_id: &str,
        ssh_host_id: &str,
        remote_cwd: &str,
    ) -> Result<nomifun_ai_agent::SshSessionBinding, String> {
        let id = SshHostId::parse(ssh_host_id)
            .map_err(|e| format!("invalid ssh_host_id: {e}"))?;
        let link = self
            .acquire(user_id, conversation_id, &id, remote_cwd)
            .await
            .map_err(|e| e.to_string())?;
        Ok(nomifun_ai_agent::SshSessionBinding {
            backend: self.backend_for(&link),
            lease: Arc::new(PooledSessionLease {
                pool: self.clone(),
                link,
            }),
        })
    }
}

/// One agent runtime's claim on a pooled link.
///
/// Holds the pool strongly on purpose: the pool is a process-level service that
/// outlives every runtime, and a lease that could not reach it would have to
/// report a failure it cannot actually distinguish from success.
struct PooledSessionLease {
    pool: SshConnectionPool,
    link: Arc<SshLink>,
}

#[async_trait::async_trait]
impl nomifun_ai_agent::SshSessionLease for PooledSessionLease {
    /// Report, never close. The runtime that holds this lease is destroyed and
    /// rebuilt on every model switch; closing here would cost the operator their
    /// shell, their cwd and another passphrase prompt for a UI click. So the answer
    /// is read out of the link's own state — the single truth this crate keeps.
    async fn release(&self) -> nomifun_ai_agent::SshLeaseRelease {
        use nomifun_ai_agent::SshLeaseRelease;
        match self.link.state() {
            SshLinkState::Closed { teardown } => match teardown {
                SshTeardown::Reaped { detail } => SshLeaseRelease::Reaped { detail },
                // A lease reports proof, and "already down" is the absence of it:
                // the remote shell was never seen to exit. The pool's shutdown
                // report keeps the two apart because it counts links for an
                // operator; a runtime can only say "proven" or "not proven".
                SshTeardown::Lost { detail } | SshTeardown::AlreadyDown { detail } => {
                    SshLeaseRelease::Lost { detail }
                }
            },
            other if self.pool.is_pooled(self.link.key()) => SshLeaseRelease::Retained {
                detail: format!(
                    "link kept for this conversation ({:?})",
                    other.phase()
                ),
            },
            other => SshLeaseRelease::Lost {
                detail: format!(
                    "link left the pool while {:?} and never reported how it closed",
                    other.phase()
                ),
            },
        }
    }
}

impl PoolInner {
    fn session_admission_for(&self, agent_session_id: &str) -> Arc<ActionAdmission> {
        Arc::clone(
            self.session_admissions
                .entry(agent_session_id.to_owned())
                .or_insert_with(|| Arc::new(ActionAdmission::default()))
                .value(),
        )
    }

    fn keys_where(&self, predicate: impl Fn(&SshLinkKey) -> bool) -> Vec<SshLinkKey> {
        // Materialized before any await: holding a DashMap guard across a suspend
        // point is how this kind of map deadlocks.
        self.links
            .iter()
            .filter(|entry| predicate(entry.key()))
            .map(|entry| entry.key().clone())
            .collect()
    }

    fn links_where(&self, predicate: impl Fn(&SshLinkKey) -> bool) -> Vec<Arc<SshLink>> {
        self.links
            .iter()
            .filter(|entry| predicate(entry.key()))
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    fn start_link_teardown(self: &Arc<Self>, link: Arc<SshLink>) {
        if !link.close.try_start() {
            return;
        }
        let pool = Arc::clone(self);
        tokio::spawn(async move {
            // This task, rather than any HTTP/action caller, owns the close. A
            // cancelled waiter merely stops waiting; teardown continues and its
            // terminal receipt remains available to the next caller.
            link.actions.wait_drained().await;
            let teardown = pool.tear_down(&link).await;
            pool.teardown_receipts.record(
                TeardownReceiptKey {
                    link: link.key.clone(),
                    instance_id: link.instance_id,
                },
                teardown.clone(),
            );
            link.close.finish(teardown);
            // No await may occur between publishing the terminal receipt and
            // removing the map entry. Consequently, map absence never means a
            // teardown is merely in progress.
            pool.links
                .remove_if(&link.key, |_, current| Arc::ptr_eq(current, &link));
        });
    }

    fn latest_teardown_receipt(
        &self,
        key: &SshLinkKey,
    ) -> Option<(TeardownReceiptKey, SshTeardown)> {
        self.teardown_receipts.latest_for_link(key)
    }

    fn teardown_receipts_for_session(
        &self,
        agent_session_id: &str,
    ) -> Vec<(TeardownReceiptKey, SshTeardown)> {
        self.teardown_receipts.for_session(agent_session_id)
    }

    fn acknowledge_persisted_teardowns(&self, agent_session_id: &str) -> usize {
        self.teardown_receipts
            .acknowledge_persisted_for_session(agent_session_id)
    }

    fn link_for(&self, key: &SshLinkKey, owner_id: &str, remote_cwd: &str) -> Arc<SshLink> {
        if let Some(existing) = self.links.get(key) {
            return Arc::clone(existing.value());
        }
        let created = Arc::new(SshLink::new(
            self.next_link_instance.fetch_add(1, Ordering::Relaxed),
            key.clone(),
            owner_id,
            remote_cwd,
        ));
        // `entry`, not `insert`: two turns of the same conversation may race to
        // bind the session, and both must end up with the same link.
        Arc::clone(self.links.entry(key.clone()).or_insert(created).value())
    }

    /// The only writer of a link's state.
    ///
    /// Publishing and emitting in one call is what keeps the socket's truth and
    /// the operator's screen from drifting. Nothing is emitted unless the state
    /// actually changed: the realtime channel is shared per user, and an idle
    /// session whose liveness tick found everything fine must stay silent.
    fn publish(&self, link: &SshLink, next: SshLinkState) {
        let at = nomifun_common::now_ms();
        let changed = link.state_tx.send_if_modified(|current| {
            if *current == next {
                false
            } else {
                *current = next.clone();
                // Stamped inside the closure, so it is already visible when the
                // watch wakes a subscriber: a client that reacts to the change by
                // re-reading the snapshot must not find the old timestamp on the
                // new state.
                link.changed_at.store(at, Ordering::Relaxed);
                true
            }
        });
        if !changed {
            return;
        }
        debug!(
            conversation_id = %link.key.conversation_id,
            ssh_host_id = %link.key.ssh_host_id,
            phase = ?next.phase(),
            "ssh link state changed"
        );
        let event = SshStatusEvent::from_state(
            link.key.ssh_host_id.as_str(),
            &link.key.conversation_id,
            &next,
            at,
        );
        self.events.emit_status(&link.owner_id, &event);
    }

    async fn ensure_connected(self: &Arc<Self>, link: &Arc<SshLink>) -> Result<(), SshDialError> {
        if link.has_live_transport().await {
            return Ok(());
        }
        let _turn = link.transition.lock().await;
        // Someone may have dialled while we queued for the lock.
        if link.has_live_transport().await {
            return Ok(());
        }
        self.publish(link, SshLinkState::Connecting { attempt: 1 });
        match self.dial(link, DialIntent::Fresh).await {
            Ok(handle) => {
                self.adopt(link, handle).await;
                Ok(())
            }
            Err(e) => {
                self.fail(link, &e).await;
                Err(e)
            }
        }
    }

    /// Install a freshly dialled handle and let everyone know.
    async fn adopt(self: &Arc<Self>, link: &Arc<SshLink>, handle: SshConnectionHandle) {
        let fingerprint = handle.fingerprint.clone();
        *link.handle.write().await = Some(Arc::new(handle));
        self.publish(
            link,
            SshLinkState::Connected {
                fingerprint: fingerprint.clone(),
            },
        );
        self.spawn_supervisor(link);
        // Best-effort hint on the host row; the live truth is the watch above.
        if let Err(e) = self
            .service
            .mark_connected(
                &link.owner_id,
                &link.key.ssh_host_id,
                fingerprint.as_deref(),
            )
            .await
        {
            debug!(error = %e, "could not stamp the ssh host as connected");
        }
    }

    async fn fail(&self, link: &Arc<SshLink>, err: &SshDialError) {
        let detail = err.to_string();
        self.publish(
            link,
            SshLinkState::Dropped {
                detail: detail.clone(),
                retryable: err.is_retryable(),
            },
        );
        if let Err(e) = self
            .service
            .mark_unreachable(&link.owner_id, &link.key.ssh_host_id, &detail)
            .await
        {
            debug!(error = %e, "could not walk back the ssh host status");
        }
    }

    /// Dial the link's host through its gate, replaying the link's last proven
    /// cwd so a reconnect lands where the model left off.
    async fn dial(
        &self,
        link: &Arc<SshLink>,
        intent: DialIntent,
    ) -> Result<SshConnectionHandle, SshDialError> {
        self.dial_host(
            &link.owner_id,
            &link.key.ssh_host_id,
            &link.last_cwd(),
            intent,
        )
        .await
    }

    /// One dial, serialized per host and answered from the gate when a very recent
    /// attempt already knows how it ends.
    async fn dial_host(
        &self,
        user_id: &str,
        host_id: &SshHostId,
        cwd: &str,
        intent: DialIntent,
    ) -> Result<SshConnectionHandle, SshDialError> {
        let gate = self.gate_for(host_id);
        if let Some(err) = gate.blocked(intent) {
            return Err(err);
        }
        let _queue = gate.dialing.lock().await;
        // Whoever held the gate ahead of us may have just learned something
        // terminal about this host.
        if let Some(err) = gate.blocked(intent) {
            return Err(err);
        }

        let outcome = match self.service.decrypt_credential(user_id, host_id).await {
            // Bounded here and nowhere else: the transport has no connect or
            // handshake timeout, and this await is held under the per-host dial
            // lock, so an unbounded one blocks every other session's `acquire` on
            // this host — the agent's included — not just the caller's.
            Ok(cred) => match tokio::time::timeout(
                SSH_DIAL_TIMEOUT,
                SshConnectionHandle::connect(cred, self.known_hosts.clone(), cwd),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(SshDialError::Unreachable(format!(
                    "ssh dial timed out after {}s",
                    SSH_DIAL_TIMEOUT.as_secs()
                ))),
            },
            Err(e) => Err(SshDialError::from(e)),
        };
        match &outcome {
            Ok(_) => gate.clear(),
            Err(e) => gate.record_failure(e.clone(), self.tuning.dial_cooldown),
        }
        outcome
    }

    fn gate_for(&self, host_id: &SshHostId) -> Arc<HostGate> {
        Arc::clone(
            self.gates
                .entry(host_id.clone())
                .or_insert_with(|| Arc::new(HostGate::new()))
                .value(),
        )
    }

    /// The shell is unusable but the transport is not: rebuild the channels on the
    /// same session instead of redialling (which would cost a handshake and
    /// re-touch `known_hosts` for nothing).
    async fn recycle_shell(&self, link: &Arc<SshLink>, detail: &str) {
        let _turn = link.transition.lock().await;
        let Some(stale) = link.current_handle().await else {
            return;
        };
        if stale.is_transport_closed() {
            // Not a wedged shell after all — the socket is gone, and redialling is
            // the ladder's job, not ours.
            return;
        }
        self.publish(
            link,
            SshLinkState::Degraded {
                detail: detail.to_string(),
            },
        );

        let cwd = link.last_cwd();
        match stale.reopen_channels(&cwd).await {
            Ok(fresh) => {
                let fingerprint = fresh.fingerprint.clone();
                *link.handle.write().await = Some(Arc::new(fresh));
                // The stale shell is wedged, so its close is unlikely to be
                // provable; we only want its channel off the transport.
                let proof = stale.shell().close(self.tuning.close_budget).await;
                debug!(reaped = proof.is_reaped(), "closed the recycled remote shell");
                self.publish(link, SshLinkState::Connected { fingerprint });
            }
            Err(e) => {
                *link.handle.write().await = None;
                let retryable = e.is_retryable();
                self.publish(
                    link,
                    SshLinkState::Dropped {
                        detail: e.to_string(),
                        retryable,
                    },
                );
                link.wake.notify_one();
            }
        }
    }

    /// A tool call found the transport gone. Drop the dead handle so no further
    /// call lands on it and wake the supervisor, rather than waiting out a whole
    /// liveness interval with a session the model believes is up.
    async fn note_transport_loss(&self, link: &Arc<SshLink>, detail: &str) {
        {
            let mut slot = link.handle.write().await;
            // Only act on a handle that really is dead: by the time a failed call
            // gets here the supervisor may already have installed a live one.
            if !slot.as_ref().is_some_and(|h| h.is_transport_closed()) {
                return;
            }
            *slot = None;
        }
        if matches!(
            link.state(),
            SshLinkState::Connected { .. } | SshLinkState::Degraded { .. }
        ) {
            self.publish(
                link,
                SshLinkState::Dropped {
                    detail: detail.to_string(),
                    retryable: true,
                },
            );
        }
        link.wake.notify_one();
    }

    /// Close a link that has already been removed from the map, and publish what
    /// could be proven.
    async fn tear_down(&self, link: &Arc<SshLink>) -> SshTeardown {
        // Stop the supervisor *before* queueing for the transition lock: it may be
        // asleep on a backoff rung while holding that lock, and shutdown must not
        // have to wait out a ladder. Aborting at an await point drops its guard.
        self.abort_supervisor(&link.key);

        let teardown = match tokio::time::timeout(
            self.tuning.close_budget,
            link.transition.lock(),
        )
        .await
        {
            Ok(_turn) => self.reap(link).await,
            Err(_) => SshTeardown::Lost {
                detail: "link busy; could not take it over to close it".to_string(),
            },
        };
        self.publish(
            link,
            SshLinkState::Closed {
                teardown: teardown.clone(),
            },
        );
        teardown
    }

    /// Take the handle away and try to prove the remote shell died.
    async fn reap(&self, link: &Arc<SshLink>) -> SshTeardown {
        let Some(handle) = link.handle.write().await.take() else {
            return SshTeardown::AlreadyDown {
                detail: "the link was already down".to_string(),
            };
        };
        if handle.is_transport_closed() {
            return SshTeardown::AlreadyDown {
                detail: "the transport was already gone; nothing left to reap".to_string(),
            };
        }
        let proof = handle.shell().close(self.tuning.close_budget).await;
        let teardown = SshTeardown::from_proof(&proof);
        // Say goodbye properly so the server logs a deliberate disconnect rather
        // than a torn-down TCP connection.
        if let Err(e) = handle.conn().disconnect().await {
            debug!(error = %e, "ssh disconnect message could not be sent");
        }
        teardown
    }

    fn abort_supervisor(&self, key: &SshLinkKey) {
        if let Some((_, supervisor)) = self.supervisors.remove(key) {
            supervisor.abort();
        }
    }

    /// Give a link a supervisor, unless it already has a live one.
    fn spawn_supervisor(self: &Arc<Self>, link: &Arc<SshLink>) {
        // A link that has left the map is being torn down; giving it a supervisor
        // now would leave a task nobody holds the handle to. (One that slips
        // through anyway exits on its first round, because the state is `Closed`.)
        if !self.links.contains_key(&link.key) || link.actions.is_retired() {
            return;
        }
        // A supervisor that has *finished* — the ladder ran out, or a terminal
        // failure stopped it — leaves its handle behind. Treating that as "already
        // supervised" would mean a link revived by a later `acquire` never got
        // watched again.
        if self
            .supervisors
            .get(&link.key)
            .is_some_and(|supervisor| !supervisor.is_finished())
        {
            return;
        }
        let pool = Arc::downgrade(self);
        let watched = Arc::clone(link);
        let supervisor = tokio::spawn(async move { supervise(pool, watched).await });
        self.supervisors.insert(link.key.clone(), supervisor);
    }

    /// One supervision round. Returns false when this link no longer needs a
    /// supervisor.
    async fn supervise_once(self: &Arc<Self>, link: &Arc<SshLink>) -> bool {
        if self.quiescing.load(Ordering::SeqCst) {
            return false;
        }
        match link.state() {
            SshLinkState::Closed { .. } => return false,
            // The ladder gave up, or a terminal failure stopped it. The link is
            // inert until someone acquires it again — and `ensure_connected`
            // respawns a supervisor then — so stop burning a timer on it.
            SshLinkState::Dropped {
                retryable: false, ..
            } => return false,
            _ => {}
        }

        if let Some(handle) = link.current_handle().await {
            if self.transport_is_alive(&handle).await {
                // Nothing changed, so nothing is published: an idle session must
                // not heartbeat the shared realtime channel.
                return true;
            }
            {
                let mut slot = link.handle.write().await;
                // Probing took a round trip, in which an `acquire` or a recycle may
                // have installed a better handle. Clearing the slot blindly would
                // throw theirs away and leak the socket behind it, so only the
                // handle we actually condemned may be removed.
                let condemned_is_current = slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &handle));
                if !condemned_is_current {
                    return true;
                }
                // Taken away *before* anything is announced, so a tool call
                // arriving now is told the link is down instead of being handed a
                // dead shell.
                *slot = None;
            }
            self.publish(
                link,
                SshLinkState::Dropped {
                    detail: "the ssh transport went away".to_string(),
                    retryable: true,
                },
            );
        }
        self.climb_the_ladder(link).await
    }

    /// Liveness in two steps, because neither step alone is enough.
    ///
    /// `is_transport_closed` is free and takes no channel lock, so it still answers
    /// while a long command holds the shell — but it only reflects our own session
    /// task, so a black-holed link looks perfectly idle to it. The round trip
    /// closes that hole, and we pay for it on *every* tick: a remote session that
    /// has silently stopped working is the worst failure mode here, and the cost is
    /// one keepalive per link per interval — the same order as OpenSSH's own
    /// `ServerAliveInterval`, which doubles as a NAT keepalive.
    ///
    /// The budget is explicit because russh 0.62.5's `send_ping` discards the pong
    /// (`let _ = receiver.await`): a missing reply can only be observed as our own
    /// timeout elapsing, never as a returned error.
    async fn transport_is_alive(&self, handle: &Arc<SshConnectionHandle>) -> bool {
        if handle.is_transport_closed() {
            return false;
        }
        if tokio::time::timeout(self.tuning.ping_timeout, handle.conn().ping())
            .await
            .is_err()
        {
            return false;
        }
        // The ping also returns Ok when the session task vanished underneath it, so
        // ask the cheap bit again now that a round trip has gone by.
        !handle.is_transport_closed()
    }

    /// Walk the backoff ladder until the link is back, the attempts run out, or a
    /// terminal failure stops us. Returns false when the supervisor should stop.
    async fn climb_the_ladder(self: &Arc<Self>, link: &Arc<SshLink>) -> bool {
        for attempt in 1..=self.tuning.max_attempts {
            let wait = self.tuning.delay(attempt);
            self.publish(
                link,
                SshLinkState::Reconnecting {
                    attempt,
                    next_retry_in_ms: wait.as_millis() as u64,
                },
            );
            tokio::time::sleep(wait).await;
            if self.quiescing.load(Ordering::SeqCst) {
                return false;
            }

            // Taken per attempt rather than around the whole climb: an `acquire`
            // arriving mid-outage must be able to answer its caller instead of
            // queueing behind a ladder that can run for minutes.
            let _turn = link.transition.lock().await;
            if link.has_live_transport().await {
                // An `acquire` beat us to it while we were waiting.
                return true;
            }
            if matches!(link.state(), SshLinkState::Closed { .. }) {
                return false;
            }

            self.publish(link, SshLinkState::Connecting { attempt });
            match self.dial(link, DialIntent::Redial).await {
                Ok(handle) => {
                    self.adopt(link, handle).await;
                    return true;
                }
                Err(e) if !e.is_retryable() => {
                    // Rejected credentials, a changed host key, a withdrawn host:
                    // retrying would only lock the account out or re-accept a key
                    // that a human has to look at first.
                    self.fail(link, &e).await;
                    return false;
                }
                Err(e) => debug!(attempt, error = %e, "ssh redial failed"),
            }
        }
        self.publish(
            link,
            SshLinkState::Dropped {
                detail: format!(
                    "gave up after {} reconnect attempts",
                    self.tuning.max_attempts
                ),
                retryable: true,
            },
        );
        false
    }
}

/// One supervisor per link. Holds only a `Weak` on the pool, so a dropped pool
/// ends its supervisors instead of keeping them — and the links they watch — alive
/// for the rest of the process.
async fn supervise(pool: std::sync::Weak<PoolInner>, link: Arc<SshLink>) {
    loop {
        let Some(inner) = pool.upgrade() else {
            return;
        };
        let poll = inner.tuning.liveness_poll;
        drop(inner);

        // A tool call that just found the link dead wakes us instead of leaving the
        // operator staring at a "connected" pill for the rest of the interval.
        tokio::select! {
            _ = tokio::time::sleep(poll) => {}
            _ = link.wake.notified() => {}
        }

        let Some(inner) = pool.upgrade() else {
            return;
        };
        if !inner.supervise_once(&link).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn close_completion_is_shared_and_outlives_a_cancelled_waiter() {
        let completion = Arc::new(LinkCloseCompletion::default());
        assert!(completion.try_start(), "one owner must claim teardown");
        assert!(!completion.try_start(), "a second close must share the owner");

        let release_owner = Arc::new(Notify::new());
        let owner = {
            let completion = Arc::clone(&completion);
            let release_owner = Arc::clone(&release_owner);
            tokio::spawn(async move {
                release_owner.notified().await;
                completion.finish(SshTeardown::Lost {
                    detail: "synthetic close proof unavailable".to_owned(),
                });
            })
        };
        let cancelled_waiter = {
            let completion = Arc::clone(&completion);
            tokio::spawn(async move { completion.wait().await })
        };
        let concurrent_waiter = {
            let completion = Arc::clone(&completion);
            tokio::spawn(async move { completion.wait().await })
        };
        tokio::task::yield_now().await;
        cancelled_waiter.abort();
        cancelled_waiter
            .await
            .expect_err("the first request waiter is intentionally cancelled");

        release_owner.notify_one();
        owner.await.expect("the process-owned close task must finish");
        let shared = concurrent_waiter
            .await
            .expect("the concurrent waiter must receive the close receipt");
        assert!(matches!(shared, SshTeardown::Lost { .. }));
        assert_eq!(completion.wait().await, shared, "late retry gets the same receipt");
    }

    #[test]
    fn teardown_receipts_are_retry_stable_per_incarnation_until_durable_ack() {
        let receipts = TeardownReceipts::default();
        let host = SshHostId::new();
        let link = SshLinkKey::new("session-a", host.clone());
        let first = TeardownReceiptKey {
            link: link.clone(),
            instance_id: 1,
        };
        let second = TeardownReceiptKey {
            link: link.clone(),
            instance_id: 2,
        };
        receipts.record(
            first,
            SshTeardown::Lost {
                detail: "first incarnation lost".to_owned(),
            },
        );
        receipts.record(
            second.clone(),
            SshTeardown::Lost {
                detail: "second incarnation lost".to_owned(),
            },
        );
        let proven = TeardownReceiptKey {
            link: SshLinkKey::new("session-a", SshHostId::new()),
            instance_id: 3,
        };
        receipts.record(
            proven.clone(),
            SshTeardown::Reaped {
                detail: "exit status 0".to_owned(),
            },
        );

        let (latest, teardown) = receipts
            .latest_for_link(&link)
            .expect("an empty active-link sweep must still find Lost");
        assert_eq!(latest, second);
        assert!(matches!(teardown, SshTeardown::Lost { .. }));
        assert_eq!(receipts.for_session("session-a").len(), 3);

        assert_eq!(receipts.for_session("session-a").len(), 3);
        assert_eq!(receipts.acknowledge_persisted_for_session("session-a"), 3);
        assert!(receipts.for_session("session-a").is_empty());
    }

    #[tokio::test]
    async fn retirement_rejects_new_work_and_waits_for_the_linearized_winner() {
        let admission = Arc::new(ActionAdmission::default());
        let winner = admission.admit().expect("the first action is admitted");

        admission.retire();
        assert!(
            admission.admit().is_none(),
            "work arriving after the retirement fence must be rejected"
        );

        let waiting = {
            let admission = Arc::clone(&admission);
            tokio::spawn(async move { admission.wait_drained().await })
        };
        tokio::task::yield_now().await;
        assert!(
            !waiting.is_finished(),
            "cleanup must wait for the action that won admission"
        );

        drop(winner);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("the drain should wake when the terminal lease drops")
            .expect("the drain task should not panic");
    }

    #[test]
    fn default_tuning_reproduces_the_pinned_ladder() {
        // The literal ladder, spelled out: the tests shrink these numbers, and
        // production must not silently inherit a different ladder. This is the
        // only place the 1s→60s sequence is pinned, so it is written as expected
        // values rather than recomputed from the same constants it is checking.
        let expected_ms = [
            1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000, 60_000, 60_000, 60_000,
            60_000,
        ];
        let tuning = PoolTuning::default();
        for (i, want) in expected_ms.iter().enumerate() {
            let attempt = (i + 1) as u32;
            assert_eq!(
                tuning.delay(attempt),
                Duration::from_millis(*want),
                "attempt {attempt}"
            );
        }
        assert_eq!(tuning.max_attempts, SSH_RECONNECT_MAX_ATTEMPTS);
        assert_eq!(tuning.liveness_poll, SSH_LIVENESS_POLL_INTERVAL);
        assert_eq!(tuning.close_budget, SSH_CLOSE_BUDGET);
    }

    #[test]
    fn a_cooling_gate_answers_without_a_second_attempt() {
        let gate = HostGate::new();
        assert!(gate.blocked(DialIntent::Fresh).is_none());
        gate.record_failure(
            SshDialError::Unreachable("refused".into()),
            Duration::from_secs(30),
        );
        assert!(
            gate.blocked(DialIntent::Fresh).is_some(),
            "a fresh failure must spare the network a second attempt"
        );
        assert!(gate.blocked(DialIntent::Redial).is_some());
        gate.clear();
        assert!(gate.blocked(DialIntent::Fresh).is_none());
    }

    #[test]
    fn a_retired_host_stays_shut_to_supervisors_and_opens_for_an_operator() {
        let gate = HostGate::new();
        gate.retire(SshDialError::Credential("withdrawn".into()));
        assert!(
            gate.blocked(DialIntent::Redial).is_some(),
            "a supervisor must never dial a withdrawn host"
        );
        assert!(
            gate.blocked(DialIntent::Fresh).is_none(),
            "an operator asking again clears the retirement"
        );
        assert!(
            gate.blocked(DialIntent::Redial).is_none(),
            "and the clearing sticks, so the revived host is supervised normally"
        );
    }

    fn cooling_until(gate: &HostGate) -> Instant {
        match &*gate.block.lock().unwrap() {
            Some(GateBlock::Cooling { until, .. }) => *until,
            _ => panic!("expected a cooling gate"),
        }
    }

    #[test]
    fn a_terminal_failure_closes_the_gate_for_longer_than_a_transient_one() {
        let cooldown = Duration::from_millis(100);
        let transient = HostGate::new();
        transient.record_failure(SshDialError::Unreachable("refused".into()), cooldown);
        let terminal = HostGate::new();
        terminal.record_failure(SshDialError::Auth("rejected".into()), cooldown);

        assert!(
            cooling_until(&terminal) > cooling_until(&transient),
            "a rejected credential must not be retried as eagerly as a refused port"
        );
    }
}
