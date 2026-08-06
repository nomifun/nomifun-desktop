use std::{
    collections::VecDeque,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const POLLER_STACK_BYTES: usize = 512 * 1024;
const BLOCKING_WORKERS: usize = 4;
const IDLE_POLL: Duration = Duration::from_millis(25);
const MIN_CAPACITY: usize = 128;
const MAX_CAPACITY: usize = 2_048;
const CAPACITY_PER_CPU: usize = 64;

/// Result of one short, non-blocking platform lifecycle poll.
///
/// Jobs stay owned by the singleton poller until they either produce exact
/// process-tree cleanup proof or explicitly quarantine themselves as
/// unproven. `Quarantine` is deliberately not completion: the whole job (and
/// therefore every exact OS authority it still owns) remains retained and its
/// admission slot stays consumed.
pub(crate) enum LifecyclePoll {
    Pending { next_poll: Instant },
    /// The job reached terminal cleanup work which may perform bounded OS
    /// waits. It is moved to the fixed worker pool; live-process waiting must
    /// never use this variant.
    Blocking,
    ExactComplete,
    Quarantine { reason: String },
}

pub(crate) trait LifecyclePollJob: Send + 'static {
    /// Run one bounded, non-blocking state-machine step.
    fn poll(&mut self, now: Instant) -> LifecyclePoll;

    /// Run terminal, bounded blocking cleanup on one of the fixed workers.
    fn poll_blocking(&mut self) -> LifecyclePoll {
        LifecyclePoll::Quarantine {
            reason: format!("{} requested unsupported blocking cleanup", self.label()),
        }
    }

    /// Publish an infrastructure failure to any waiter before the job is
    /// retained in quarantine. Implementations must not discard OS authority.
    fn poller_failed(&mut self, reason: &str);

    fn label(&self) -> &'static str;
}

struct ScheduledJob {
    next_poll: Instant,
    job: Box<dyn LifecyclePollJob>,
}

struct PollerQueues {
    pending: VecDeque<ScheduledJob>,
    blocking: VecDeque<Box<dyn LifecyclePollJob>>,
    results: VecDeque<(Box<dyn LifecyclePollJob>, LifecyclePoll)>,
    quarantined: Vec<Box<dyn LifecyclePollJob>>,
}

impl PollerQueues {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            blocking: VecDeque::new(),
            results: VecDeque::new(),
            quarantined: Vec::new(),
        }
    }
}

#[derive(Default)]
struct PollerCounters {
    admitted: AtomicUsize,
    /// Independently bounds boxed lifecycle jobs. One admitted process
    /// authority may legitimately be shared by several cleanup fragments, so
    /// this cannot be derived from `admitted`.
    retained_jobs: AtomicUsize,
    pending: AtomicUsize,
    quarantined: AtomicUsize,
    blocking_active: AtomicUsize,
    workers: AtomicUsize,
    running: AtomicBool,
    submitted: AtomicU64,
    completed: AtomicU64,
    polls: AtomicU64,
    panics: AtomicU64,
    overflow_retained: AtomicU64,
}

struct PollerShared {
    capacity: usize,
    queues: Mutex<PollerQueues>,
    wake: Condvar,
    shutdown: AtomicBool,
    counters: PollerCounters,
}

#[derive(Clone)]
pub(crate) struct PlatformLifecyclePermit {
    _lease: Arc<PlatformLifecycleLease>,
}

struct PlatformLifecycleLease {
    shared: Arc<PollerShared>,
}

impl PlatformLifecyclePermit {
    #[cfg(test)]
    pub(crate) fn test_capacity(&self) -> usize {
        self._lease.shared.capacity
    }
}

impl Drop for PlatformLifecycleLease {
    fn drop(&mut self) {
        let previous = self.shared.counters.admitted.fetch_sub(1, Ordering::AcqRel);
        if previous == 0 {
            self.shared.counters.admitted.store(0, Ordering::Release);
            tracing::error!("platform lifecycle admission underflow");
        }
        self.shared.wake.notify_all();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlatformLifecycleMetrics {
    pub capacity: usize,
    /// Spawn transactions plus live, cleaning, and quarantined authorities.
    pub admitted: usize,
    /// Boxed jobs across pending, worker, result, and quarantine ownership.
    pub retained_jobs: usize,
    pub pending: usize,
    pub quarantined: usize,
    /// The singleton OS poller thread is either absent or exactly one.
    pub poller_threads: usize,
    pub blocking_workers: usize,
    pub blocking_active: usize,
    pub submitted: u64,
    pub completed: u64,
    pub polls: u64,
    pub panics: u64,
    /// Jobs retained outside the bounded containers after an internal
    /// submission-contract violation. Their exact authority is intentionally
    /// leaked instead of being dropped unsafely.
    pub overflow_retained: u64,
}

pub(crate) struct PlatformLifecyclePoller {
    shared: Arc<PollerShared>,
}

impl PlatformLifecyclePoller {
    fn start() -> io::Result<Self> {
        let capacity = lifecycle_capacity();
        let shared = Arc::new(PollerShared {
            capacity,
            queues: Mutex::new(PollerQueues::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            counters: PollerCounters::default(),
        });
        let worker_shared = Arc::clone(&shared);
        let poller_thread = std::thread::Builder::new()
            .name("nomi-platform-lifecycle-poller".to_owned())
            .stack_size(POLLER_STACK_BYTES)
            .spawn(move || run_poller(worker_shared))?;
        let mut worker_threads = Vec::with_capacity(BLOCKING_WORKERS);
        for worker_id in 0..BLOCKING_WORKERS {
            let worker_shared = Arc::clone(&shared);
            match std::thread::Builder::new()
                .name(format!("nomi-platform-cleanup-{worker_id}"))
                .stack_size(POLLER_STACK_BYTES)
                .spawn(move || run_blocking_worker(worker_shared))
            {
                Ok(worker) => worker_threads.push(worker),
                Err(error) => {
                    shared.shutdown.store(true, Ordering::Release);
                    shared.wake.notify_all();
                    let _ = poller_thread.join();
                    for worker in worker_threads {
                        let _ = worker.join();
                    }
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "start platform lifecycle cleanup worker {worker_id}: {error}"
                        ),
                    ));
                }
            }
        }
        shared
            .counters
            .workers
            .store(worker_threads.len(), Ordering::Release);
        shared.counters.running.store(true, Ordering::Release);
        // Successful singleton workers intentionally live for the process
        // lifetime. Dropping JoinHandles detaches them without multiplying
        // any later spawn-specific threads.
        drop(poller_thread);
        drop(worker_threads);
        Ok(Self { shared })
    }

    /// Reserve platform cleanup capacity before any physical process or
    /// watchdog is created. A saturated debt budget therefore rejects the
    /// spawn without creating another OS resource.
    pub(crate) fn reserve(&self) -> io::Result<PlatformLifecyclePermit> {
        if !self.shared.counters.running.load(Ordering::Acquire) {
            return Err(io::Error::other(
                "platform lifecycle poller is unavailable; refusing physical spawn",
            ));
        }
        let admitted = self
            .shared
            .counters
            .admitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < self.shared.capacity).then_some(current + 1)
            })
            .map_err(|current| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "platform lifecycle authority budget is saturated ({current}/{})",
                        self.shared.capacity
                    ),
                )
            })?;
        debug_assert!(admitted < self.shared.capacity);
        Ok(PlatformLifecyclePermit {
            _lease: Arc::new(PlatformLifecycleLease {
                shared: Arc::clone(&self.shared),
            }),
        })
    }

    /// Submit an already-admitted job. Submission itself cannot allocate an
    /// unbounded queue: at most `capacity` permits exist process-wide.
    pub(crate) fn submit(
        &self,
        job: Box<dyn LifecyclePollJob>,
    ) -> Result<(), Box<dyn LifecyclePollJob>> {
        if !self.shared.counters.running.load(Ordering::Acquire) {
            return Err(job);
        }
        if !try_retain_job_slot(&self.shared) {
            return Err(job);
        }
        if !self.shared.counters.running.load(Ordering::Acquire) {
            release_retained_job_slot(&self.shared);
            return Err(job);
        }
        let mut queues = self
            .shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queues.pending.push_back(ScheduledJob {
            next_poll: Instant::now(),
            job,
        });
        self.shared
            .counters
            .pending
            .store(queues.pending.len(), Ordering::Release);
        self.shared
            .counters
            .submitted
            .fetch_add(1, Ordering::Relaxed);
        drop(queues);
        // The dispatcher and fixed blocking workers share one condition
        // variable. `notify_one` could wake an idle cleanup worker (which has
        // no blocking job yet) while leaving the dispatcher asleep forever
        // with a newly submitted poll job.
        self.shared.wake.notify_all();
        Ok(())
    }

    pub(crate) fn quarantine_unscheduled(
        &self,
        mut job: Box<dyn LifecyclePollJob>,
        reason: &str,
    ) {
        notify_poller_failed(&self.shared, job.as_mut(), reason);
        if !try_retain_job_slot(&self.shared) {
            // A cloned permit can manufacture more cleanup fragments than the
            // process-authority admission count. Never let that turn into an
            // unbounded queue and never drop exact OS authority on overflow.
            self.shared
                .counters
                .overflow_retained
                .fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                job = safe_job_label(&self.shared, job.as_ref()),
                %reason,
                "platform lifecycle hard job cap reached; authority retained outside queues"
            );
            std::mem::forget(job);
            return;
        }
        let mut queues = self
            .shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queues.quarantined.push(job);
        self.shared
            .counters
            .quarantined
            .store(queues.quarantined.len(), Ordering::Release);
        self.shared.wake.notify_all();
    }

    fn metrics(&self) -> PlatformLifecycleMetrics {
        let counters = &self.shared.counters;
        PlatformLifecycleMetrics {
            capacity: self.shared.capacity,
            admitted: counters.admitted.load(Ordering::Acquire),
            retained_jobs: counters.retained_jobs.load(Ordering::Acquire),
            pending: counters.pending.load(Ordering::Acquire),
            quarantined: counters.quarantined.load(Ordering::Acquire),
            poller_threads: usize::from(counters.running.load(Ordering::Acquire)),
            blocking_workers: counters.workers.load(Ordering::Acquire),
            blocking_active: counters.blocking_active.load(Ordering::Acquire),
            submitted: counters.submitted.load(Ordering::Acquire),
            completed: counters.completed.load(Ordering::Acquire),
            polls: counters.polls.load(Ordering::Acquire),
            panics: counters.panics.load(Ordering::Acquire),
            overflow_retained: counters.overflow_retained.load(Ordering::Acquire),
        }
    }
}

static PLATFORM_POLLER: OnceLock<Result<PlatformLifecyclePoller, Arc<str>>> = OnceLock::new();

pub(crate) fn platform_lifecycle_poller() -> io::Result<&'static PlatformLifecyclePoller> {
    match PLATFORM_POLLER.get_or_init(|| {
        PlatformLifecyclePoller::start().map_err(|error| Arc::<str>::from(error.to_string()))
    }) {
        Ok(poller) => Ok(poller),
        Err(error) => Err(io::Error::other(format!(
            "platform lifecycle poller initialization failed: {error}"
        ))),
    }
}

pub fn platform_lifecycle_metrics() -> PlatformLifecycleMetrics {
    match PLATFORM_POLLER.get() {
        Some(Ok(poller)) => poller.metrics(),
        _ => PlatformLifecycleMetrics {
            capacity: lifecycle_capacity(),
            ..PlatformLifecycleMetrics::default()
        },
    }
}

fn lifecycle_capacity() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .saturating_mul(CAPACITY_PER_CPU)
        .clamp(MIN_CAPACITY, MAX_CAPACITY)
}

fn try_retain_job_slot(shared: &PollerShared) -> bool {
    shared
        .counters
        .retained_jobs
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < shared.capacity).then_some(current + 1)
        })
        .is_ok()
}

fn release_retained_job_slot(shared: &PollerShared) {
    let previous = shared
        .counters
        .retained_jobs
        .fetch_sub(1, Ordering::AcqRel);
    if previous == 0 {
        shared
            .counters
            .retained_jobs
            .store(0, Ordering::Release);
        shared.counters.running.store(false, Ordering::Release);
        tracing::error!("platform lifecycle retained-job accounting underflow");
    }
    shared.wake.notify_all();
}

fn safe_job_label(shared: &PollerShared, job: &dyn LifecyclePollJob) -> &'static str {
    match catch_unwind(AssertUnwindSafe(|| job.label())) {
        Ok(label) => label,
        Err(_) => {
            shared.counters.panics.fetch_add(1, Ordering::Relaxed);
            "panicking-platform-lifecycle-job"
        }
    }
}

/// Failure notification is advisory; exact authority retention is not. A
/// broken callback must never unwind through the dispatcher or drop its box.
fn notify_poller_failed(shared: &PollerShared, job: &mut dyn LifecyclePollJob, reason: &str) {
    if catch_unwind(AssertUnwindSafe(|| job.poller_failed(reason))).is_err() {
        shared.counters.panics.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            job = safe_job_label(shared, job),
            %reason,
            "platform lifecycle failure callback panicked; authority remains retained"
        );
    }
}

fn run_poller(shared: Arc<PollerShared>) {
    let _liveness = PollerLivenessGuard {
        shared: Arc::clone(&shared),
    };
    loop {
        let scheduled = {
            let mut queues = shared
                .queues
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if shared.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some((job, outcome)) = queues.results.pop_front() {
                    shared
                        .counters
                        .blocking_active
                        .fetch_sub(1, Ordering::AcqRel);
                    drop(queues);
                    settle_outcome(&shared, job, outcome);
                    queues = shared
                        .queues
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    continue;
                }
                if queues.pending.is_empty() {
                    queues = shared
                        .wake
                        .wait(queues)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    continue;
                }
                let now = Instant::now();
                let mut earliest = now + IDLE_POLL;
                let round = queues.pending.len();
                let mut due = None;
                for _ in 0..round {
                    let candidate = queues
                        .pending
                        .pop_front()
                        .expect("platform poller queue length is stable while locked");
                    if due.is_none() && candidate.next_poll <= now {
                        due = Some(candidate);
                        break;
                    }
                    earliest = earliest.min(candidate.next_poll);
                    queues.pending.push_back(candidate);
                }
                if let Some(due) = due {
                    shared
                        .counters
                        .pending
                        .store(queues.pending.len(), Ordering::Release);
                    break due;
                }
                let wait = earliest
                    .saturating_duration_since(Instant::now())
                    .min(IDLE_POLL);
                queues = shared
                    .wake
                    .wait_timeout(queues, wait)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
        };

        let ScheduledJob { mut job, .. } = scheduled;
        shared.counters.polls.fetch_add(1, Ordering::Relaxed);
        let outcome = match catch_unwind(AssertUnwindSafe(|| job.poll(Instant::now()))) {
            Ok(outcome) => outcome,
            Err(_) => {
                shared.counters.panics.fetch_add(1, Ordering::Relaxed);
                LifecyclePoll::Quarantine {
                    reason: "platform lifecycle job panicked; exact authority retained".to_owned(),
                }
            }
        };
        if let LifecyclePoll::Quarantine { reason } = &outcome {
            notify_poller_failed(&shared, job.as_mut(), reason);
            tracing::error!(
                job = safe_job_label(&shared, job.as_ref()),
                %reason,
                "platform lifecycle authority quarantined"
            );
        }
        let mut queues = shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match outcome {
            LifecyclePoll::Pending { next_poll } => {
                queues.pending.push_back(ScheduledJob { next_poll, job });
            }
            LifecyclePoll::ExactComplete => {
                shared
                    .counters
                    .completed
                    .fetch_add(1, Ordering::Relaxed);
                drop(queues);
                // Exact proof has already removed the OS risk. Release the
                // boxed-job slot before Drop releases the process admission
                // lease so a newly admitted authority cannot observe a
                // transient false job-cap saturation.
                release_retained_job_slot(&shared);
                if catch_unwind(AssertUnwindSafe(|| drop(job))).is_err() {
                    shared.counters.panics.fetch_add(1, Ordering::Relaxed);
                    tracing::error!("completed platform lifecycle job destructor panicked");
                }
                continue;
            }
            LifecyclePoll::Blocking => {
                queues.blocking.push_back(job);
                shared
                    .counters
                    .blocking_active
                    .fetch_add(1, Ordering::AcqRel);
                shared.wake.notify_all();
            }
            LifecyclePoll::Quarantine { .. } => {
                queues.quarantined.push(job);
            }
        }
        shared
            .counters
            .pending
            .store(queues.pending.len(), Ordering::Release);
        shared
            .counters
            .quarantined
            .store(queues.quarantined.len(), Ordering::Release);
    }
}

fn settle_outcome(
    shared: &Arc<PollerShared>,
    mut job: Box<dyn LifecyclePollJob>,
    outcome: LifecyclePoll,
) {
    if let LifecyclePoll::Quarantine { reason } = &outcome {
        notify_poller_failed(shared, job.as_mut(), reason);
        tracing::error!(
            job = safe_job_label(shared, job.as_ref()),
            %reason,
            "platform lifecycle authority quarantined"
        );
    }
    let mut queues = shared
        .queues
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match outcome {
        LifecyclePoll::Pending { next_poll } => {
            queues.pending.push_back(ScheduledJob {
                next_poll,
                job,
            });
        }
        LifecyclePoll::Blocking => {
            // A bounded terminal worker may need another bounded pass (for
            // example after a short kill-on-close grace). Requeue it fairly.
            queues.pending.push_back(ScheduledJob {
                next_poll: Instant::now() + IDLE_POLL,
                job,
            });
        }
        LifecyclePoll::ExactComplete => {
            shared.counters.completed.fetch_add(1, Ordering::Relaxed);
            drop(queues);
            release_retained_job_slot(shared);
            if catch_unwind(AssertUnwindSafe(|| drop(job))).is_err() {
                shared.counters.panics.fetch_add(1, Ordering::Relaxed);
                tracing::error!("completed blocking lifecycle job destructor panicked");
            }
            return;
        }
        LifecyclePoll::Quarantine { .. } => {
            queues.quarantined.push(job);
        }
    }
    shared
        .counters
        .pending
        .store(queues.pending.len(), Ordering::Release);
    shared
        .counters
        .quarantined
        .store(queues.quarantined.len(), Ordering::Release);
}

fn run_blocking_worker(shared: Arc<PollerShared>) {
    let _liveness = WorkerLivenessGuard {
        shared: Arc::clone(&shared),
    };
    loop {
        let mut job = {
            let mut queues = shared
                .queues
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if shared.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some(job) = queues.blocking.pop_front() {
                    break job;
                }
                queues = shared
                    .wake
                    .wait(queues)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| job.poll_blocking())).unwrap_or_else(|_| {
            shared.counters.panics.fetch_add(1, Ordering::Relaxed);
            LifecyclePoll::Quarantine {
                reason: "platform blocking cleanup job panicked; exact authority retained"
                    .to_owned(),
            }
        });
        let mut queues = shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queues.results.push_back((job, outcome));
        drop(queues);
        shared.wake.notify_all();
    }
}

struct PollerLivenessGuard {
    shared: Arc<PollerShared>,
}

impl Drop for PollerLivenessGuard {
    fn drop(&mut self) {
        self.shared.counters.running.store(false, Ordering::Release);
        self.shared.wake.notify_all();
    }
}

struct WorkerLivenessGuard {
    shared: Arc<PollerShared>,
}

impl Drop for WorkerLivenessGuard {
    fn drop(&mut self) {
        if self.shared.shutdown.load(Ordering::Acquire) {
            self.shared.wake.notify_all();
            return;
        }
        let previous = self.shared.counters.workers.fetch_sub(1, Ordering::AcqRel);
        if previous <= 1 {
            self.shared.counters.running.store(false, Ordering::Release);
        }
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_shared(capacity: usize) -> Arc<PollerShared> {
        let shared = Arc::new(PollerShared {
            capacity,
            queues: Mutex::new(PollerQueues::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            counters: PollerCounters::default(),
        });
        shared.counters.running.store(true, Ordering::Release);
        shared
    }

    fn wait_for_counter(counter: &AtomicUsize, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::Acquire) != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(counter.load(Ordering::Acquire), expected);
    }

    struct ExactTestJob {
        _permit: PlatformLifecyclePermit,
        completed: Arc<AtomicUsize>,
    }

    impl LifecyclePollJob for ExactTestJob {
        fn poll(&mut self, _now: Instant) -> LifecyclePoll {
            self.completed.fetch_add(1, Ordering::AcqRel);
            LifecyclePoll::ExactComplete
        }

        fn poller_failed(&mut self, _reason: &str) {}

        fn label(&self) -> &'static str {
            "exact-test-job"
        }
    }

    struct AuthorityDropProbe(Arc<AtomicUsize>);

    impl Drop for AuthorityDropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[derive(Clone, Copy)]
    enum PanicFailureMode {
        Direct,
        Blocking,
    }

    struct PanicFailureJob {
        _permit: PlatformLifecyclePermit,
        _authority: AuthorityDropProbe,
        mode: PanicFailureMode,
    }

    impl LifecyclePollJob for PanicFailureJob {
        fn poll(&mut self, _now: Instant) -> LifecyclePoll {
            match self.mode {
                PanicFailureMode::Direct => LifecyclePoll::Quarantine {
                    reason: "direct quarantine test".to_owned(),
                },
                PanicFailureMode::Blocking => LifecyclePoll::Blocking,
            }
        }

        fn poll_blocking(&mut self) -> LifecyclePoll {
            LifecyclePoll::Quarantine {
                reason: "blocking quarantine test".to_owned(),
            }
        }

        fn poller_failed(&mut self, _reason: &str) {
            panic!("injected poller_failed callback panic");
        }

        fn label(&self) -> &'static str {
            "panic-failure-test-job"
        }
    }

    struct StickyTestJob {
        _permit: PlatformLifecyclePermit,
        _authority: AuthorityDropProbe,
    }

    impl LifecyclePollJob for StickyTestJob {
        fn poll(&mut self, now: Instant) -> LifecyclePoll {
            LifecyclePoll::Pending {
                next_poll: now + Duration::from_secs(1),
            }
        }

        fn poller_failed(&mut self, _reason: &str) {}

        fn label(&self) -> &'static str {
            "sticky-test-job"
        }
    }

    #[test]
    fn adaptive_capacity_is_finite_and_nonzero() {
        let capacity = lifecycle_capacity();
        assert!((MIN_CAPACITY..=MAX_CAPACITY).contains(&capacity));
    }

    #[test]
    fn admission_is_hard_bounded_before_submission() {
        let shared = test_shared(2);
        let poller = PlatformLifecyclePoller {
            shared: Arc::clone(&shared),
        };
        let first = poller.reserve().expect("first authority");
        let first_cleanup_fragment = first.clone();
        let second = poller.reserve().expect("second authority");
        assert_eq!(first.test_capacity(), 2);
        assert_eq!(second.test_capacity(), 2);
        assert_eq!(
            poller.reserve().err().expect("third authority is fenced").kind(),
            io::ErrorKind::WouldBlock
        );
        drop(first);
        assert_eq!(
            poller
                .reserve()
                .err()
                .expect("shared lease still owns the slot")
                .kind(),
            io::ErrorKind::WouldBlock,
            "dropping one clone must not release a still-owned platform authority"
        );
        drop(first_cleanup_fragment);
        assert!(poller.reserve().is_ok(), "released proof capacity must recover");
    }

    #[test]
    fn permanent_debt_fences_n_plus_one_and_cloned_fragments_cannot_grow_queues() {
        let shared = test_shared(2);
        let poller = PlatformLifecyclePoller {
            shared: Arc::clone(&shared),
        };
        let authority_drops = Arc::new(AtomicUsize::new(0));
        let first = poller.reserve().expect("first physical authority");
        let cloned_fragment = first.clone();
        let second = poller.reserve().expect("second physical authority");
        poller.quarantine_unscheduled(
            Box::new(StickyTestJob {
                _permit: first,
                _authority: AuthorityDropProbe(Arc::clone(&authority_drops)),
            }),
            "first permanent debt",
        );
        poller.quarantine_unscheduled(
            Box::new(StickyTestJob {
                _permit: second,
                _authority: AuthorityDropProbe(Arc::clone(&authority_drops)),
            }),
            "second permanent debt",
        );
        assert_eq!(shared.counters.retained_jobs.load(Ordering::Acquire), 2);
        assert_eq!(shared.counters.quarantined.load(Ordering::Acquire), 2);
        assert_eq!(
            poller
                .reserve()
                .err()
                .expect("N+1 physical spawn must fail closed")
                .kind(),
            io::ErrorKind::WouldBlock
        );

        // One process permit can be cloned for legitimate failure fragments,
        // but those fragments have their own hard resident-job ceiling.
        poller.quarantine_unscheduled(
            Box::new(StickyTestJob {
                _permit: cloned_fragment,
                _authority: AuthorityDropProbe(Arc::clone(&authority_drops)),
            }),
            "overflowing cloned cleanup fragment",
        );
        let queues = shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(queues.quarantined.len(), 2);
        assert_eq!(shared.counters.retained_jobs.load(Ordering::Acquire), 2);
        assert_eq!(shared.counters.overflow_retained.load(Ordering::Acquire), 1);
        assert_eq!(authority_drops.load(Ordering::Acquire), 0);
    }

    #[test]
    fn unscheduled_failure_callback_panic_keeps_authority_quarantined() {
        let shared = test_shared(2);
        let poller = PlatformLifecyclePoller {
            shared: Arc::clone(&shared),
        };
        let authority_drops = Arc::new(AtomicUsize::new(0));
        let permit = poller.reserve().expect("test authority");
        poller.quarantine_unscheduled(
            Box::new(PanicFailureJob {
                _permit: permit,
                _authority: AuthorityDropProbe(Arc::clone(&authority_drops)),
                mode: PanicFailureMode::Direct,
            }),
            "unscheduled test failure",
        );

        assert_eq!(shared.counters.quarantined.load(Ordering::Acquire), 1);
        assert_eq!(shared.counters.retained_jobs.load(Ordering::Acquire), 1);
        assert_eq!(authority_drops.load(Ordering::Acquire), 0);
        assert!(shared.counters.running.load(Ordering::Acquire));
        assert_eq!(shared.counters.panics.load(Ordering::Acquire), 1);
    }

    #[test]
    fn direct_quarantine_callback_panic_does_not_kill_dispatcher_or_drop_authority() {
        let shared = test_shared(2);
        let poller = PlatformLifecyclePoller {
            shared: Arc::clone(&shared),
        };
        let authority_drops = Arc::new(AtomicUsize::new(0));
        let dispatcher_shared = Arc::clone(&shared);
        let dispatcher = std::thread::spawn(move || run_poller(dispatcher_shared));
        let permit = poller.reserve().expect("test authority");
        poller
            .submit(Box::new(PanicFailureJob {
                _permit: permit,
                _authority: AuthorityDropProbe(Arc::clone(&authority_drops)),
                mode: PanicFailureMode::Direct,
            }))
            .unwrap_or_else(|_| panic!("test job must be submitted"));

        wait_for_counter(&shared.counters.quarantined, 1);
        assert_eq!(shared.counters.retained_jobs.load(Ordering::Acquire), 1);
        assert_eq!(authority_drops.load(Ordering::Acquire), 0);
        assert!(shared.counters.running.load(Ordering::Acquire));
        assert_eq!(shared.counters.panics.load(Ordering::Acquire), 1);

        shared.shutdown.store(true, Ordering::Release);
        shared.wake.notify_all();
        dispatcher.join().expect("dispatcher exits after test shutdown");
    }

    #[test]
    fn blocking_quarantine_callback_panic_does_not_kill_workers_or_drop_authority() {
        let shared = test_shared(2);
        shared.counters.workers.store(1, Ordering::Release);
        let poller = PlatformLifecyclePoller {
            shared: Arc::clone(&shared),
        };
        let authority_drops = Arc::new(AtomicUsize::new(0));
        let dispatcher_shared = Arc::clone(&shared);
        let dispatcher = std::thread::spawn(move || run_poller(dispatcher_shared));
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || run_blocking_worker(worker_shared));
        let permit = poller.reserve().expect("test authority");
        poller
            .submit(Box::new(PanicFailureJob {
                _permit: permit,
                _authority: AuthorityDropProbe(Arc::clone(&authority_drops)),
                mode: PanicFailureMode::Blocking,
            }))
            .unwrap_or_else(|_| panic!("test job must be submitted"));

        wait_for_counter(&shared.counters.quarantined, 1);
        assert_eq!(shared.counters.retained_jobs.load(Ordering::Acquire), 1);
        assert_eq!(authority_drops.load(Ordering::Acquire), 0);
        assert!(shared.counters.running.load(Ordering::Acquire));
        assert_eq!(shared.counters.panics.load(Ordering::Acquire), 1);

        shared.shutdown.store(true, Ordering::Release);
        shared.wake.notify_all();
        dispatcher.join().expect("dispatcher exits after test shutdown");
        worker.join().expect("worker exits after test shutdown");
    }

    #[test]
    fn many_jobs_share_one_poller_and_a_fixed_worker_pool() {
        let poller = platform_lifecycle_poller().expect("singleton poller starts");
        let completed = Arc::new(AtomicUsize::new(0));
        const JOBS: usize = 64;
        for _ in 0..JOBS {
            let permit = poller.reserve().expect("stress job is admitted");
            poller
                .submit(Box::new(ExactTestJob {
                    _permit: permit,
                    completed: Arc::clone(&completed),
                }))
                .unwrap_or_else(|_| panic!("admitted stress job must enter the bounded queue"));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while completed.load(Ordering::Acquire) != JOBS && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(completed.load(Ordering::Acquire), JOBS);
        let metrics = platform_lifecycle_metrics();
        assert_eq!(metrics.poller_threads, 1);
        assert_eq!(metrics.blocking_workers, BLOCKING_WORKERS);
        assert!(metrics.pending <= metrics.capacity);
        assert!(metrics.admitted <= metrics.capacity);
        assert!(metrics.retained_jobs <= metrics.capacity);
    }
}
