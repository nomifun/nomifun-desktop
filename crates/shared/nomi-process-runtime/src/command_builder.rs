use std::{
    cell::Cell,
    collections::VecDeque,
    ffi::{OsStr, OsString},
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    },
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::fd::{OwnedFd, RawFd};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

struct CancelChildOutputOnDrop {
    cancellation: CancellationToken,
    armed: bool,
}

impl Drop for CancelChildOutputOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}


/// Exact completion proof for the platform process-tree authority attached to
/// one [`ChildProcessBuilder`] spawn.
///
/// The direct child can exit before its descendants. Awaiting Tokio's
/// `Child::wait` alone is therefore not a lifecycle boundary: Unix still has a
/// watchdog-owned process group and Windows still has a process Job to seal.
/// This handle is captured at spawn time (before PID reuse is possible) and
/// resolves only after that platform authority proves the whole tree empty.
#[derive(Clone)]
pub struct ChildProcessCleanup {
    #[cfg(unix)]
    inner: crate::platform::unix::ChildProcessCleanup,
    #[cfg(windows)]
    inner: crate::platform::windows::ChildProcessCleanup,
}

impl ChildProcessCleanup {
    /// Consume this completion proof and wait until the platform authority has
    /// proved the whole process tree empty.
    pub async fn wait(self) -> io::Result<()> {
        self.wait_ref().await
    }

    async fn wait_ref(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            return self.inner.wait().await;
        }
        #[cfg(windows)]
        {
            return self.inner.wait().await;
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(())
        }
    }

    async fn shutdown(&self, child: &mut Child) -> io::Result<()> {
        #[cfg(unix)]
        {
            return self.inner.shutdown(child).await;
        }
        #[cfg(windows)]
        {
            return self.inner.shutdown(child).await;
        }
        #[cfg(not(any(unix, windows)))]
        {
            child.kill().await?;
            child.wait().await.map(|_| ())
        }
    }
}

/// Exact, single-owner lifecycle authority for one managed child process tree.
///
/// This value deliberately owns both the Tokio direct-child handle and the
/// platform cleanup proof. Callers must use [`Self::shutdown`] rather than
/// waiting/terminating those proofs independently: one operation requests
/// whole-tree termination, reaps the direct child, and proves platform tree
/// cleanup. Failed or cancelled attempts leave the same authority available
/// for a later retry.
pub struct ManagedChildProcess {
    child: Option<Child>,
    cleanup: Option<ChildProcessCleanup>,
    shutdown_complete: bool,
}

const MANAGED_CLEANUP_SYNC_GRACE: Duration = Duration::from_millis(500);
const MANAGED_CLEANUP_RELAY_CAPACITY: usize = 64;
const MANAGED_CLEANUP_RELAY_WORKERS: usize = 4;
const MANAGED_CLEANUP_THREAD_STACK_BYTES: usize = 512 * 1024;
const MANAGED_CLEANUP_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const MANAGED_CLEANUP_ADMISSION_WAIT: Duration = Duration::from_millis(500);
const MANAGED_CLEANUP_MAX_ATTEMPTS: u32 = 20;
const MANAGED_CLEANUP_RETRY_INITIAL: Duration = Duration::from_millis(250);
const MANAGED_CLEANUP_RETRY_MAX: Duration = Duration::from_secs(30);
const MANAGED_CLEANUP_DISPATCH_TICK: Duration = Duration::from_millis(25);

/// Observable state for the process-local managed-child cleanup relay.
///
/// `retained` counts exact cleanup authorities currently owned by this generic
/// relay. The relay never admits more than `capacity`; saturation waits only a
/// bounded interval before returning ownership to the independent platform
/// Job/watchdog reaper registered at spawn time. Thus one failing workload
/// cannot permanently block unrelated `ManagedChildProcess::drop` callers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ManagedChildCleanupMetrics {
    /// Maximum number of exact cleanup authorities admitted at once.
    pub capacity: usize,
    /// All admitted authorities that have not completed exact cleanup.
    pub retained: usize,
    /// Authorities waiting for a worker, including an enqueue in progress.
    pub queued: usize,
    /// Cleanup attempts currently executing.
    pub active: usize,
    /// Failed authorities sleeping until their next automatic retry.
    pub delayed: usize,
    /// Live fixed cleanup workers.
    pub workers: usize,
    /// Whether the singleton dispatcher accepts new authorities.
    pub running: bool,
    /// Managed-child drops observed by this relay.
    pub submitted: u64,
    /// Authorities that have proved their process tree empty.
    pub completed: u64,
    /// Failed attempts rescheduled by the dispatcher.
    pub retries: u64,
    /// Handoffs that encountered the bounded admission ceiling.
    pub saturated_handoffs: u64,
    /// Handoffs that could not use the generic relay and invoked its bounded
    /// fallback path. This does not claim platform proof completion.
    pub inline_fallbacks: u64,
    /// Authorities handed back to the already-running platform Job/watchdog
    /// reaper after the generic relay could not make bounded progress.
    pub platform_handoffs: u64,
}

#[derive(Default)]
struct ManagedCleanupRelayState {
    retained: AtomicUsize,
    active: AtomicUsize,
    delayed: AtomicUsize,
    workers: AtomicUsize,
    submitted: AtomicU64,
    completed: AtomicU64,
    retries: AtomicU64,
    saturated_handoffs: AtomicU64,
    inline_fallbacks: AtomicU64,
    platform_handoffs: AtomicU64,
    running: AtomicBool,
}

struct ManagedCleanupAdmission {
    retained: Mutex<usize>,
    available: Condvar,
    capacity: usize,
    state: Arc<ManagedCleanupRelayState>,
}

impl ManagedCleanupAdmission {
    fn new(capacity: usize, state: Arc<ManagedCleanupRelayState>) -> Self {
        Self {
            retained: Mutex::new(0),
            available: Condvar::new(),
            capacity,
            state,
        }
    }

    /// Reserve one of the relay's bounded ownership slots. A saturated relay
    /// applies short fail-closed backpressure, then lets the caller transfer to
    /// the already-registered platform reaper rather than block indefinitely.
    fn acquire(&self) -> bool {
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut saturation_recorded = false;
        let deadline = Instant::now() + MANAGED_CLEANUP_ADMISSION_WAIT;
        while *retained >= self.capacity {
            if !saturation_recorded {
                self.state
                    .saturated_handoffs
                    .fetch_add(1, Ordering::Relaxed);
                saturation_recorded = true;
            }
            if !self.state.running.load(Ordering::Acquire) {
                return false;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            retained = self
                .available
                .wait_timeout(retained, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        if !self.state.running.load(Ordering::Acquire) {
            return false;
        }
        *retained += 1;
        self.state.retained.store(*retained, Ordering::Release);
        true
    }

    fn release(&self) {
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *retained == 0 {
            tracing::error!("managed child cleanup relay admission underflow");
            return;
        }
        *retained -= 1;
        self.state.retained.store(*retained, Ordering::Release);
        self.available.notify_one();
    }
}

struct ManagedCleanupRelay {
    sender: Option<SyncSender<ManagedCleanupJob>>,
    admission: Arc<ManagedCleanupAdmission>,
    state: Arc<ManagedCleanupRelayState>,
}

struct ManagedCleanupJob {
    process: ManagedChildProcess,
    attempts: u32,
    ready_at: Instant,
    _admission: ManagedCleanupAdmissionPermit,
}

struct ManagedCleanupAdmissionPermit {
    admission: Arc<ManagedCleanupAdmission>,
}

impl Drop for ManagedCleanupAdmissionPermit {
    fn drop(&mut self) {
        self.admission.release();
    }
}

struct ManagedCleanupWorkerResult {
    worker_id: usize,
    job: Option<ManagedCleanupJob>,
    error: Option<String>,
}

struct ManagedCleanupWorker {
    sender: SyncSender<ManagedCleanupJob>,
    busy: bool,
    alive: bool,
}

struct ManagedCleanupDispatcherGuard {
    admission: Arc<ManagedCleanupAdmission>,
    state: Arc<ManagedCleanupRelayState>,
}

impl Drop for ManagedCleanupDispatcherGuard {
    fn drop(&mut self) {
        self.state.running.store(false, Ordering::Release);
        self.admission.available.notify_all();
    }
}

struct ManagedCleanupWorkerGuard {
    state: Arc<ManagedCleanupRelayState>,
    active: bool,
}

impl Drop for ManagedCleanupWorkerGuard {
    fn drop(&mut self) {
        if self.active {
            self.state.active.fetch_sub(1, Ordering::AcqRel);
        }
        self.state.workers.fetch_sub(1, Ordering::AcqRel);
    }
}

static MANAGED_CLEANUP_RELAY: OnceLock<ManagedCleanupRelay> = OnceLock::new();
thread_local! {
    /// Prevent a relay-thread unwind from recursively enqueueing its retained
    /// jobs back into the dispatcher that is currently unwinding.
    static IN_MANAGED_CLEANUP_RELAY_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Return a point-in-time cleanup-relay snapshot without starting the relay.
pub fn managed_child_cleanup_metrics() -> ManagedChildCleanupMetrics {
    let Some(relay) = MANAGED_CLEANUP_RELAY.get() else {
        return ManagedChildCleanupMetrics {
            capacity: MANAGED_CLEANUP_RELAY_CAPACITY,
            ..ManagedChildCleanupMetrics::default()
        };
    };
    let state = &relay.state;
    let retained = state.retained.load(Ordering::Acquire);
    let active = state.active.load(Ordering::Acquire);
    let delayed = state.delayed.load(Ordering::Acquire);
    ManagedChildCleanupMetrics {
        capacity: relay.admission.capacity,
        retained,
        queued: retained.saturating_sub(active.saturating_add(delayed)),
        active,
        delayed,
        workers: state.workers.load(Ordering::Acquire),
        running: state.running.load(Ordering::Acquire),
        submitted: state.submitted.load(Ordering::Acquire),
        completed: state.completed.load(Ordering::Acquire),
        retries: state.retries.load(Ordering::Acquire),
        saturated_handoffs: state.saturated_handoffs.load(Ordering::Acquire),
        inline_fallbacks: state.inline_fallbacks.load(Ordering::Acquire),
        platform_handoffs: state.platform_handoffs.load(Ordering::Acquire),
    }
}

impl ManagedChildProcess {
    fn from_parts(child: Child, cleanup: ChildProcessCleanup) -> Self {
        Self {
            child: Some(child),
            cleanup: Some(cleanup),
            shutdown_complete: false,
        }
    }

    pub fn child(&self) -> &Child {
        self.child
            .as_ref()
            .expect("managed child direct-child handle is unavailable")
    }

    pub fn child_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("managed child direct-child handle is unavailable")
    }

    pub fn id(&self) -> Option<u32> {
        self.child().id()
    }

    pub async fn shutdown(&mut self) -> io::Result<()> {
        if self.shutdown_complete {
            return Ok(());
        }

        let result = match (self.cleanup.as_ref(), self.child.as_mut()) {
            (Some(cleanup), Some(child)) => cleanup.shutdown(child).await,
            (None, _) => Err(io::Error::other(
                "managed child process cleanup authority is unavailable",
            )),
            (_, None) => Err(io::Error::other(
                "managed child process direct-child authority is unavailable",
            )),
        };
        match result {
            Ok(()) => {
                // The proof is linear for the managed path. Once it has
                // completed, discard it exactly once; later shutdown calls
                // return from `shutdown_complete` without another kill/wait.
                self.cleanup.take();
                self.shutdown_complete = true;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl std::ops::Deref for ManagedChildProcess {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        self.child()
    }
}

impl std::ops::DerefMut for ManagedChildProcess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child_mut()
    }
}

impl Drop for ManagedChildProcess {
    fn drop(&mut self) {
        if self.shutdown_complete {
            return;
        }
        let (Some(child), Some(cleanup)) = (self.child.take(), self.cleanup.take()) else {
            return;
        };
        let process = Self {
            child: Some(child),
            cleanup: Some(cleanup),
            shutdown_complete: false,
        };
        hand_off_managed_child_cleanup(process);
    }
}

fn hand_off_managed_child_cleanup(mut process: ManagedChildProcess) {
    if std::thread::panicking()
        && IN_MANAGED_CLEANUP_RELAY_THREAD.with(Cell::get)
    {
        if let Some(relay) = MANAGED_CLEANUP_RELAY.get() {
            relay.state.inline_fallbacks.fetch_add(1, Ordering::Relaxed);
            hand_off_managed_child_to_platform_reaper(process);
            relay.state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
            return;
        }
        hand_off_managed_child_to_platform_reaper(process);
        return;
    }
    let relay = MANAGED_CLEANUP_RELAY.get_or_init(start_managed_cleanup_relay);
    relay.state.submitted.fetch_add(1, Ordering::Relaxed);

    if relay.state.retained.load(Ordering::Acquire) >= relay.admission.capacity {
        // Reduce live OS resources before applying queue backpressure. The
        // platform cleanup proof stays attached to `process` until admitted.
        let _direct_child_reaped = best_effort_synchronous_kill_and_reap(&mut process);
    }

    let Some(sender) = relay.sender.as_ref() else {
        relay.state.inline_fallbacks.fetch_add(1, Ordering::Relaxed);
        hand_off_managed_child_to_platform_reaper(process);
        relay.state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !relay.admission.acquire() {
        relay.state.inline_fallbacks.fetch_add(1, Ordering::Relaxed);
        hand_off_managed_child_to_platform_reaper(process);
        relay.state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let job = ManagedCleanupJob {
        process,
        attempts: 0,
        ready_at: Instant::now(),
        _admission: ManagedCleanupAdmissionPermit {
            admission: Arc::clone(&relay.admission),
        },
    };
    if let Err(error) = sender.send(job) {
        relay.state.running.store(false, Ordering::Release);
        relay.admission.available.notify_all();
        relay.state.inline_fallbacks.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            "managed child cleanup relay disconnected; transferring to platform reaper"
        );
        hand_off_managed_child_to_platform_reaper(error.0.process);
        relay.state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
    }
}

fn start_managed_cleanup_relay() -> ManagedCleanupRelay {
    let state = Arc::new(ManagedCleanupRelayState::default());
    let admission = Arc::new(ManagedCleanupAdmission::new(
        MANAGED_CLEANUP_RELAY_CAPACITY,
        Arc::clone(&state),
    ));
    let (incoming_sender, incoming_receiver) =
        mpsc::sync_channel(MANAGED_CLEANUP_RELAY_CAPACITY);
    let (result_sender, result_receiver) = mpsc::sync_channel(MANAGED_CLEANUP_RELAY_WORKERS);
    let mut workers = Vec::with_capacity(MANAGED_CLEANUP_RELAY_WORKERS);

    for worker_id in 0..MANAGED_CLEANUP_RELAY_WORKERS {
        let (worker_sender, worker_receiver) = mpsc::sync_channel(1);
        let result_sender = result_sender.clone();
        let worker_state = Arc::clone(&state);
        let spawn = std::thread::Builder::new()
            .name(format!("nomi-managed-cleanup-{worker_id}"))
            .stack_size(MANAGED_CLEANUP_THREAD_STACK_BYTES)
            .spawn(move || {
                IN_MANAGED_CLEANUP_RELAY_THREAD.with(|inside| inside.set(true));
                run_managed_cleanup_worker(
                    worker_id,
                    worker_receiver,
                    result_sender,
                    worker_state,
                )
            });
        match spawn {
            Ok(_worker) => workers.push(ManagedCleanupWorker {
                sender: worker_sender,
                busy: false,
                alive: true,
            }),
            Err(error) => tracing::error!(worker_id, %error, "failed to start managed cleanup worker"),
        }
    }
    drop(result_sender);

    if workers.is_empty() {
        tracing::error!(
            "managed child cleanup relay has no worker; cleanup will use platform reapers"
        );
        return ManagedCleanupRelay {
            sender: None,
            admission,
            state,
        };
    }

    state.workers.store(workers.len(), Ordering::Release);
    let dispatcher_admission = Arc::clone(&admission);
    let dispatcher_state = Arc::clone(&state);
    let dispatcher = std::thread::Builder::new()
        .name("nomi-managed-cleanup-dispatch".to_owned())
        .stack_size(MANAGED_CLEANUP_THREAD_STACK_BYTES)
        .spawn(move || {
            IN_MANAGED_CLEANUP_RELAY_THREAD.with(|inside| inside.set(true));
            run_managed_cleanup_dispatcher(
                incoming_receiver,
                result_receiver,
                workers,
                dispatcher_admission,
                dispatcher_state,
            )
        });
    match dispatcher {
        Ok(_dispatcher) => {
            state.running.store(true, Ordering::Release);
            ManagedCleanupRelay {
                sender: Some(incoming_sender),
                admission,
                state,
            }
        }
        Err(error) => {
            tracing::error!(%error, "failed to start managed cleanup dispatcher; cleanup will use platform reapers");
            ManagedCleanupRelay {
                sender: None,
                admission,
                state,
            }
        }
    }
}

fn run_managed_cleanup_dispatcher(
    incoming: Receiver<ManagedCleanupJob>,
    results: Receiver<ManagedCleanupWorkerResult>,
    mut workers: Vec<ManagedCleanupWorker>,
    admission: Arc<ManagedCleanupAdmission>,
    state: Arc<ManagedCleanupRelayState>,
) {
    let _dispatcher_guard = ManagedCleanupDispatcherGuard {
        admission: Arc::clone(&admission),
        state: Arc::clone(&state),
    };
    let mut ready = VecDeque::new();
    let mut delayed = Vec::new();
    let mut incoming_open = true;

    loop {
        loop {
            match results.try_recv() {
                Ok(mut result) => {
                    state.active.fetch_sub(1, Ordering::AcqRel);
                    if let Some(worker) = workers.get_mut(result.worker_id) {
                        worker.busy = false;
                    }
                    if let Some(mut job) = result.job.take() {
                        job.attempts = job.attempts.saturating_add(1);
                        if job.attempts >= MANAGED_CLEANUP_MAX_ATTEMPTS {
                            tracing::error!(
                                pid = job.process.child.as_ref().and_then(Child::id),
                                attempts = job.attempts,
                                "generic managed-child cleanup exhausted bounded retries; platform Job/watchdog reaper retains final authority"
                            );
                            hand_off_managed_child_to_platform_reaper(job.process);
                            state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        job.ready_at = Instant::now() + managed_cleanup_retry_delay(job.attempts);
                        state.retries.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            pid = job.process.child.as_ref().and_then(Child::id),
                            attempts = job.attempts,
                            error = result.error.as_deref().unwrap_or("unknown cleanup failure"),
                            "managed child cleanup remains retained for automatic retry"
                        );
                        delayed.push(job);
                    } else {
                        state.completed.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if incoming_open {
            loop {
                match incoming.try_recv() {
                    Ok(job) => ready.push_back(job),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        incoming_open = false;
                        break;
                    }
                }
            }
        }

        let now = Instant::now();
        let mut index = 0;
        while index < delayed.len() {
            if delayed[index].ready_at <= now {
                ready.push_back(delayed.swap_remove(index));
            } else {
                index += 1;
            }
        }

        for worker_id in 0..workers.len() {
            if ready.is_empty() {
                break;
            }
            if workers[worker_id].busy || !workers[worker_id].alive {
                continue;
            }
            let Some(job) = ready.pop_front() else {
                break;
            };
            match workers[worker_id].sender.send(job) {
                Ok(()) => {
                    workers[worker_id].busy = true;
                    state.active.fetch_add(1, Ordering::AcqRel);
                }
                Err(error) => {
                    workers[worker_id].alive = false;
                    ready.push_front(error.0);
                    tracing::error!(worker_id, "managed cleanup worker disconnected");
                }
            }
        }
        state.delayed.store(delayed.len(), Ordering::Release);

        if state.workers.load(Ordering::Acquire) == 0 {
            state.running.store(false, Ordering::Release);
            admission.available.notify_all();
            tracing::error!("all managed cleanup workers stopped; retained cleanup cannot continue on relay");
            // This state is fail-closed: keep ownership in this thread rather
            // than dropping jobs into a disconnected channel.
            for job in ready.drain(..).chain(delayed.drain(..)) {
                hand_off_managed_child_to_platform_reaper(job.process);
                state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
            }
            break;
        }

        if !incoming_open && state.retained.load(Ordering::Acquire) == 0 {
            state.running.store(false, Ordering::Release);
            admission.available.notify_all();
            break;
        }

        match incoming.recv_timeout(MANAGED_CLEANUP_DISPATCH_TICK) {
            Ok(job) => ready.push_back(job),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => incoming_open = false,
        }
    }
}

fn run_managed_cleanup_worker(
    worker_id: usize,
    receiver: Receiver<ManagedCleanupJob>,
    results: SyncSender<ManagedCleanupWorkerResult>,
    state: Arc<ManagedCleanupRelayState>,
) {
    let mut worker_guard = ManagedCleanupWorkerGuard {
        state: Arc::clone(&state),
        active: false,
    };
    let mut runtime = build_managed_cleanup_runtime();
    while let Ok(mut job) = receiver.recv() {
        worker_guard.active = true;
        if runtime.is_none() {
            runtime = build_managed_cleanup_runtime();
        }
        let outcome = runtime
            .as_ref()
            .map_or_else(
                || Err("Tokio cleanup runtime is unavailable".to_owned()),
                |runtime| run_managed_cleanup_attempt(runtime, &mut job.process),
            );
        let result = match outcome {
            Ok(()) => ManagedCleanupWorkerResult {
                worker_id,
                job: None,
                error: None,
            },
            Err(error) => ManagedCleanupWorkerResult {
                worker_id,
                job: Some(job),
                error: Some(error),
            },
        };

        // From this point the result message owns the dispatcher's active
        // accounting. `SyncSender::send` cannot unwind; on disconnection the
        // explicit fallback below decrements it instead.
        worker_guard.active = false;
        if let Err(error) = results.send(result) {
            // The dispatcher is the only path that can release an admitted
            // slot. If it disappears, this worker finishes its current exact
            // authority synchronously and releases that slot itself.
            state.active.fetch_sub(1, Ordering::AcqRel);
            if let Some(job) = error.0.job {
                hand_off_managed_child_to_platform_reaper(job.process);
                state.platform_handoffs.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
    }
}

fn build_managed_cleanup_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            tracing::error!(%error, "failed to build managed child cleanup runtime");
            None
        }
    }
}

fn run_managed_cleanup_attempt(
    runtime: &tokio::runtime::Runtime,
    process: &mut ManagedChildProcess,
) -> Result<(), String> {
    match catch_unwind(AssertUnwindSafe(|| {
        runtime.block_on(async {
            tokio::time::timeout(MANAGED_CLEANUP_ATTEMPT_TIMEOUT, process.shutdown()).await
        })
    })) {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(error.to_string()),
        Ok(Err(_elapsed)) => Err(format!(
            "cleanup attempt exceeded {} seconds",
            MANAGED_CLEANUP_ATTEMPT_TIMEOUT.as_secs()
        )),
        Err(_panic) => Err("cleanup attempt panicked".to_owned()),
    }
}

fn managed_cleanup_retry_delay(attempts: u32) -> Duration {
    let shift = attempts.saturating_sub(1).min(16);
    MANAGED_CLEANUP_RETRY_INITIAL
        .saturating_mul(1_u32 << shift)
        .min(MANAGED_CLEANUP_RETRY_MAX)
}

/// Bound the generic relay without discarding platform cleanup ownership.
///
/// Every managed spawn has already registered an independent Windows Job or
/// Unix watchdog reaper before this value is constructed. Dropping the Tokio
/// child requests direct-child termination (`kill_on_drop(true)`), while the
/// registered platform authority continues whole-tree settlement. This path
/// intentionally does not claim proof completion; it only releases the
/// generic relay slot so unrelated healthy cleanup can continue.
fn hand_off_managed_child_to_platform_reaper(mut process: ManagedChildProcess) {
    let _ = best_effort_synchronous_kill_and_reap(&mut process);
    process.shutdown_complete = true;
    drop(process.child.take());
    drop(process.cleanup.take());
}

/// Try to make progress without Tokio while preserving the exact cleanup
/// authority for a later retry. `Child::start_kill` and `Child::try_wait` are
/// synchronous and do not require a runtime; the platform cleanup worker
/// retained by [`ChildProcessCleanup`] remains authoritative for descendants.
fn best_effort_synchronous_kill_and_reap(process: &mut ManagedChildProcess) -> bool {
    let pid = process.child.as_ref().and_then(Child::id);
    let Some(child) = process.child.as_mut() else {
        tracing::error!(pid, "managed child lost its direct-child cleanup authority");
        return false;
    };

    if let Err(error) = child.start_kill()
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(pid, %error, "synchronous managed-child kill request failed");
    }

    let deadline = Instant::now() + MANAGED_CLEANUP_SYNC_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                tracing::warn!(pid, "managed child was synchronously killed and reaped; tree proof retained for retry");
                return true;
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                tracing::warn!(pid, "managed child synchronous reap grace expired; exact cleanup retained for retry");
                return false;
            }
            Err(error) => {
                tracing::warn!(pid, %error, "synchronous managed-child reap probe failed; exact cleanup retained for retry");
                return false;
            }
        }
    }
}

/// Lower-level builder for backend adapters that own one child process.
///
/// Session-oriented Agent commands use [`crate::ProcessSupervisor`]. Adapters
/// that need raw stdio or an explicit ownership hand-off use this builder; both
/// paths share the same environment hygiene and platform process-tree setup.
pub struct ChildProcessBuilder {
    inner: Command,
    hand_off: bool,
    #[cfg(unix)]
    extra_fds: Vec<(RawFd, OwnedFd)>,
}

impl ChildProcessBuilder {
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        let mut inner = Command::new(resolve_program(program.as_ref()));
        inner.kill_on_drop(true);
        configure_platform_spawn(&mut inner);
        strip_process_environment(&mut inner);
        Self {
            inner,
            hand_off: false,
            #[cfg(unix)]
            extra_fds: Vec::new(),
        }
    }

    pub fn clean_cli<S: AsRef<OsStr>>(program: S) -> Self {
        let mut builder = Self::new(program);
        builder
            .inner
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .env("TERM", "dumb");
        builder
    }

    pub fn hand_off(&mut self) -> &mut Self {
        self.hand_off = true;
        self
    }

    #[cfg(unix)]
    pub fn inherit_fds(&mut self, mappings: Vec<(RawFd, OwnedFd)>) -> &mut Self {
        self.extra_fds.extend(mappings);
        self
    }

    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.inner.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(args);
        self
    }

    pub fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.env(key, value);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.envs(vars);
        self
    }

    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Self {
        self.inner.env_remove(key);
        self
    }

    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.inner.current_dir(dir);
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, value: T) -> &mut Self {
        self.inner.stdin(value);
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, value: T) -> &mut Self {
        self.inner.stdout(value);
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, value: T) -> &mut Self {
        self.inner.stderr(value);
        self
    }

    pub fn spawn(self) -> io::Result<Child> {
        self.spawn_with_cleanup().map(|(child, _cleanup)| child)
    }

    /// Spawn a child and return the single authoritative lifecycle owner.
    pub fn spawn_managed(self) -> io::Result<ManagedChildProcess> {
        self.spawn_with_cleanup()
            .map(|(child, cleanup)| ManagedChildProcess::from_parts(child, cleanup))
    }

    /// Spawn a child and retain the exact platform tree-cleanup proof.
    ///
    /// Lifecycle-owning adapters should prefer this over [`Self::spawn`], then
    /// await both the direct child and the returned cleanup handle before
    /// publishing an exited state.
    pub fn spawn_with_cleanup(self) -> io::Result<(Child, ChildProcessCleanup)> {
        #[allow(unused_mut)]
        let mut this = self;
        #[cfg(unix)]
        if !this.extra_fds.is_empty() {
            install_fd_shuffle(&mut this.inner, &this.extra_fds);
        }
        spawn_child_process(this.inner, this.hand_off)
    }

    pub async fn output(mut self) -> io::Result<std::process::Output> {
        self.inner.stdout(Stdio::piped()).stderr(Stdio::piped());
        let (child, cleanup) = self.spawn_with_cleanup()?;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let _worker = tokio::spawn(async move {
            let output = {
                let output = child.wait_with_output();
                tokio::pin!(output);
                tokio::select! {
                    biased;
                    _ = worker_cancellation.cancelled() => Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "child process output was cancelled",
                    )),
                    output = &mut output => output,
                }
            };
            // On cancellation the losing wait_with_output future was dropped
            // first, so Child::kill_on_drop has already initiated platform
            // teardown. The Job/watchdog completion remains the exact proof.
            let cleanup_result = cleanup.wait_ref().await;
            let result = match (output, cleanup_result) {
                (Ok(output), Ok(())) => Ok(output),
                (Err(output_error), Ok(())) => Err(output_error),
                (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
                (Err(output_error), Err(cleanup_error)) => Err(io::Error::new(
                    cleanup_error.kind(),
                    format!(
                        "{output_error}; child process tree cleanup was not proven: {cleanup_error}"
                    ),
                )),
            };
            let _ = result_tx.send(result);
        });
        let mut cancel_on_drop = CancelChildOutputOnDrop {
            cancellation,
            armed: true,
        };
        let result = result_rx.await.map_err(|_| {
            io::Error::other("child process output worker stopped before reporting cleanup")
        })?;
        cancel_on_drop.armed = false;
        result
    }

    pub fn as_std(&self) -> &std::process::Command {
        self.inner.as_std()
    }
}

impl std::fmt::Debug for ChildProcessBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChildProcessBuilder")
            .field("command", self.inner.as_std())
            .field("hand_off", &self.hand_off)
            .finish()
    }
}

impl std::fmt::Display for ChildProcessBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.inner.as_std(), formatter)
    }
}

pub fn merge_process_path<I, P>(sources: I) -> io::Result<OsString>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut paths = Vec::new();
    for path in sources {
        let path = path.as_ref();
        if !path.as_os_str().is_empty() && !paths.iter().any(|existing| existing == path) {
            paths.push(path.to_path_buf());
        }
    }
    std::env::join_paths(paths).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

/// Resolve a bare command through the current process `PATH`.
///
/// Windows additionally checks the common npm/package-manager shim suffixes
/// when `PATHEXT` is incomplete. Paths supplied by the caller are not searched.
pub fn resolve_command_path(command: &str) -> Option<PathBuf> {
    if command.is_empty() || command.contains('/') || command.contains('\\') {
        return None;
    }
    which::which(command).ok().or_else(|| windows_shim_fallback(command))
}

/// Resolve a bare command inside one exact directory without walking `PATH`.
pub fn resolve_command_in(command: &str, directory: &Path) -> Option<PathBuf> {
    if command.is_empty() || command.contains('/') || command.contains('\\') {
        return None;
    }
    let path = std::env::join_paths([directory]).ok()?;
    which::which_in(command, Some(&path), directory)
        .ok()
        .or_else(|| windows_shim_fallback_in(command, directory))
}

fn resolve_program(program: &OsStr) -> OsString {
    if let Some(program) = program.to_str()
        && let Some(path) = resolve_command_path(program)
    {
        return path.into_os_string();
    }
    program.to_os_string()
}

#[cfg(windows)]
fn windows_shim_fallback(command: &str) -> Option<PathBuf> {
    if Path::new(command).extension().is_some() {
        return None;
    }
    ["cmd", "ps1", "bat"]
        .into_iter()
        .find_map(|extension| which::which(format!("{command}.{extension}")).ok())
}

#[cfg(not(windows))]
fn windows_shim_fallback(_command: &str) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn windows_shim_fallback_in(command: &str, directory: &Path) -> Option<PathBuf> {
    if Path::new(command).extension().is_some() {
        return None;
    }
    ["cmd", "ps1", "bat"]
        .into_iter()
        .map(|extension| directory.join(format!("{command}.{extension}")))
        .find(|candidate| candidate.is_file())
}

#[cfg(not(windows))]
fn windows_shim_fallback_in(_command: &str, _directory: &Path) -> Option<PathBuf> {
    None
}

fn strip_process_environment(command: &mut Command) {
    command
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_INSPECT")
        .env_remove("NODE_DEBUG")
        .env_remove("CLAUDECODE");
}

#[cfg(unix)]
fn configure_platform_spawn(command: &mut Command) {
    command.process_group(0);
}

#[cfg(windows)]
fn configure_platform_spawn(command: &mut Command) {
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn configure_platform_spawn(_command: &mut Command) {}

#[cfg(unix)]
fn install_fd_shuffle(command: &mut Command, extra_fds: &[(RawFd, OwnedFd)]) {
    use std::os::{
        fd::AsRawFd,
        unix::process::CommandExt,
    };

    let mappings = extra_fds
        .iter()
        .map(|(target, source)| (*target, source.as_raw_fd()))
        .collect::<Vec<_>>();
    // SAFETY: the closure uses only async-signal-safe fcntl/dup2/close calls
    // and reads preallocated mappings after fork.
    unsafe {
        command.as_std_mut().pre_exec(move || {
            const MAX_FDS: usize = 16;
            if mappings.len() > MAX_FDS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "too many inherited file descriptors",
                ));
            }
            let mut temporary = [(0 as RawFd, 0 as RawFd); MAX_FDS];
            let mut minimum = 20;
            for (index, (target, source)) in mappings.iter().copied().enumerate() {
                let duplicate = libc::fcntl(source, libc::F_DUPFD, minimum);
                if duplicate < 0 {
                    return Err(io::Error::last_os_error());
                }
                minimum = duplicate + 1;
                temporary[index] = (target, duplicate);
            }
            for (target, duplicate) in temporary.iter().take(mappings.len()).copied() {
                if libc::dup2(duplicate, target) < 0 {
                    return Err(io::Error::last_os_error());
                }
                libc::close(duplicate);
            }
            Ok(())
        });
    }
}

#[cfg(unix)]
fn spawn_child_process(
    command: Command,
    hand_off: bool,
) -> io::Result<(Child, ChildProcessCleanup)> {
    crate::platform::unix::spawn_child_process(command, hand_off)
        .map(|(child, inner)| (child, ChildProcessCleanup { inner }))
}

#[cfg(windows)]
fn spawn_child_process(
    command: Command,
    hand_off: bool,
) -> io::Result<(Child, ChildProcessCleanup)> {
    crate::platform::windows::spawn_child_process(command, hand_off)
        .map(|(child, inner)| (child, ChildProcessCleanup { inner }))
}

#[cfg(not(any(unix, windows)))]
fn spawn_child_process(
    mut command: Command,
    _hand_off: bool,
) -> io::Result<(Child, ChildProcessCleanup)> {
    command
        .spawn()
        .map(|child| (child, ChildProcessCleanup {}))
}

pub async fn kill_process_tree(child: &mut Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        return crate::platform::unix::kill_process_tree(child).await;
    }
    #[cfg(windows)]
    {
        return crate::platform::windows::kill_process_tree(child).await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        child.kill().await?;
        child.wait().await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_process_path_preserves_order_and_deduplicates() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let merged = merge_process_path([
            Path::new("/priority"),
            Path::new("/inherited"),
            Path::new("/priority"),
            Path::new("/login"),
        ])
        .expect("portable PATH should join");
        let rendered = merged.to_string_lossy();

        assert_eq!(
            rendered.split(separator).collect::<Vec<_>>(),
            vec!["/priority", "/inherited", "/login"]
        );
    }

    #[test]
    fn clean_builder_strips_polluting_environment() {
        #[cfg(unix)]
        {
            let builder = ChildProcessBuilder::clean_cli("example");
            let debug = format!("{builder}");
            assert!(debug.contains("-u NODE_OPTIONS"));
            assert!(debug.contains("-u CLAUDECODE"));
            assert!(debug.contains("NO_COLOR=\"1\""));
            assert!(debug.contains("TERM=\"dumb\""));
        }
    }

    #[test]
    fn managed_cleanup_retry_backoff_is_exponential_and_bounded() {
        assert_eq!(
            managed_cleanup_retry_delay(1),
            MANAGED_CLEANUP_RETRY_INITIAL
        );
        assert_eq!(
            managed_cleanup_retry_delay(2),
            MANAGED_CLEANUP_RETRY_INITIAL * 2
        );
        assert_eq!(
            managed_cleanup_retry_delay(u32::MAX),
            MANAGED_CLEANUP_RETRY_MAX
        );
    }

    #[test]
    fn managed_cleanup_admission_is_bounded_and_backpressures() {
        let state = Arc::new(ManagedCleanupRelayState::default());
        state.running.store(true, Ordering::Release);
        let admission = Arc::new(ManagedCleanupAdmission::new(1, Arc::clone(&state)));
        assert!(admission.acquire());

        let blocked_admission = Arc::clone(&admission);
        let (started_sender, started_receiver) = mpsc::channel();
        let (acquired_sender, acquired_receiver) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            started_sender.send(()).expect("start signal receiver exists");
            let acquired = blocked_admission.acquire();
            acquired_sender
                .send(acquired)
                .expect("acquire signal receiver exists");
            if acquired {
                blocked_admission.release();
            }
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("waiter started");

        let deadline = Instant::now() + Duration::from_secs(1);
        while state.saturated_handoffs.load(Ordering::Acquire) == 0
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(state.retained.load(Ordering::Acquire), 1);
        assert!(acquired_receiver.try_recv().is_err());

        admission.release();
        assert!(
            acquired_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("capacity release wakes the blocked handoff")
        );
        waiter.join().expect("admission waiter exits");
        assert_eq!(state.retained.load(Ordering::Acquire), 0);
        assert_eq!(state.saturated_handoffs.load(Ordering::Acquire), 1);
    }

    #[test]
    fn saturated_managed_cleanup_admission_has_a_bounded_drop_wait() {
        let state = Arc::new(ManagedCleanupRelayState::default());
        state.running.store(true, Ordering::Release);
        let admission = ManagedCleanupAdmission::new(1, Arc::clone(&state));
        assert!(admission.acquire());

        let started = Instant::now();
        assert!(
            !admission.acquire(),
            "a saturated generic relay must hand authority to the platform reaper instead of blocking forever"
        );
        let elapsed = started.elapsed();
        assert!(elapsed >= MANAGED_CLEANUP_ADMISSION_WAIT);
        assert!(elapsed < Duration::from_secs(2));
        assert_eq!(state.retained.load(Ordering::Acquire), 1);
        assert_eq!(state.saturated_handoffs.load(Ordering::Acquire), 1);
        admission.release();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(managed_cleanup_relay)]
    async fn dropped_managed_child_completes_without_another_handoff() {
        #[cfg(windows)]
        let builder = {
            let mut builder = ChildProcessBuilder::new("cmd.exe");
            builder.args(["/D", "/S", "/C", "ping -n 120 127.0.0.1 >NUL"]);
            builder
        };
        #[cfg(unix)]
        let builder = {
            let mut builder = ChildProcessBuilder::new("sh");
            builder.args(["-c", "sleep 120"]);
            builder
        };
        #[cfg(not(any(unix, windows)))]
        let builder = ChildProcessBuilder::new("false");

        let before = managed_child_cleanup_metrics();
        let process = builder
            .spawn_managed()
            .expect("long-lived managed child should spawn");
        assert!(process.id().is_some());
        drop(process);

        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let metrics = managed_child_cleanup_metrics();
                if metrics.submitted >= before.submitted + 1
                    && metrics.completed >= before.completed + 1
                    && metrics.retained <= before.retained
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the singleton relay must progress without a future Drop trigger");
    }
}
