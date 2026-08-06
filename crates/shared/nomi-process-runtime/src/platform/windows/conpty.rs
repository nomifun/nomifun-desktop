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

use windows_sys::{
    Win32::{
        System::Console::{
            COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
        },
    },
    core::HRESULT,
};

use super::handles::OwnedHandle;

pub(super) const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const CLOSE_WORKERS: usize = 4;
const CLOSE_WORKER_STACK_BYTES: usize = 512 * 1024;
const MIN_CLOSE_CAPACITY: usize = 32;
const MAX_CLOSE_CAPACITY: usize = 512;
const CLOSE_CAPACITY_PER_CPU: usize = 16;
static CLOSE_EXECUTOR: OnceLock<Result<ConPtyCloseExecutor, Arc<str>>> = OnceLock::new();

struct CloseJob {
    handle: HPCON,
    action: CloseAction,
    completion: Arc<CloseCompletion>,
    _authority: ConPtyClosePermit,
}

#[derive(Clone)]
enum CloseAction {
    System,
    #[cfg(test)]
    Test(Arc<dyn Fn(HPCON) -> io::Result<()> + Send + Sync>),
}

impl CloseAction {
    fn run(&self, handle: HPCON) -> io::Result<()> {
        match self {
            Self::System => {
                // SAFETY: the job owns the sole live HPCON and invokes this
                // consuming API exactly once.
                unsafe { ClosePseudoConsole(handle) };
                Ok(())
            }
            #[cfg(test)]
            Self::Test(close) => close(handle),
        }
    }
}

type CloseResult = Result<(), Arc<str>>;

struct CloseCompletion {
    result: Mutex<Option<CloseResult>>,
    wake: Condvar,
}

impl CloseCompletion {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            wake: Condvar::new(),
        }
    }

    fn publish(&self, result: CloseResult) {
        let mut current = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.is_none() {
            *current = Some(result);
        }
        self.wake.notify_all();
    }

    fn wait_until(&self, deadline: Instant) -> io::Result<()> {
        let mut result = self
            .result
            .lock()
            .map_err(|_| io::Error::other("pseudoconsole close completion is poisoned"))?;
        loop {
            if let Some(result) = result.as_ref() {
                return result
                    .clone()
                    .map_err(|message| io::Error::other(message.to_string()));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ClosePseudoConsole did not finish before the bounded deadline; the fixed close executor retains sole ownership",
                ));
            }
            let waited = self
                .wake
                .wait_timeout(result, remaining)
                .map_err(|_| io::Error::other("pseudoconsole close completion is poisoned"))?;
            result = waited.0;
            if waited.1.timed_out() && result.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ClosePseudoConsole did not finish before the bounded deadline; the fixed close executor retains sole ownership",
                ));
            }
        }
    }
}

struct CloseQueues {
    pending: VecDeque<CloseJob>,
    quarantined: Vec<CloseJob>,
}

impl CloseQueues {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            quarantined: Vec::new(),
        }
    }
}

impl Drop for CloseQueues {
    fn drop(&mut self) {
        // Executor teardown must never turn unfinished or unproven close work
        // into an HPCON Drop. Exact ownership is intentionally retained for
        // the remaining process lifetime.
        for job in self.pending.drain(..) {
            std::mem::forget(job);
        }
        for job in self.quarantined.drain(..) {
            std::mem::forget(job);
        }
    }
}

#[derive(Default)]
struct CloseCounters {
    admitted: AtomicUsize,
    retained_jobs: AtomicUsize,
    pending: AtomicUsize,
    active: AtomicUsize,
    quarantined: AtomicUsize,
    workers: AtomicUsize,
    running: AtomicBool,
    completed: AtomicU64,
    panics: AtomicU64,
    overflow_retained: AtomicU64,
    peak_pending: AtomicUsize,
}

struct CloseShared {
    capacity: usize,
    queues: Mutex<CloseQueues>,
    wake: Condvar,
    shutdown: AtomicBool,
    counters: CloseCounters,
}

struct CloseExecutorInner {
    shared: Arc<CloseShared>,
    workers: Mutex<Option<Vec<std::thread::JoinHandle<()>>>>,
}

impl Drop for CloseExecutorInner {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        let workers = self
            .workers
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        for worker in workers {
            let _ = worker.join();
        }
    }
}

#[derive(Clone)]
struct ConPtyCloseExecutor {
    inner: Arc<CloseExecutorInner>,
}

struct ConPtyClosePermit {
    shared: Arc<CloseShared>,
}

impl Drop for ConPtyClosePermit {
    fn drop(&mut self) {
        let previous = self
            .shared
            .counters
            .admitted
            .fetch_sub(1, Ordering::AcqRel);
        if previous == 0 {
            self.shared
                .counters
                .admitted
                .store(0, Ordering::Release);
            self.shared.counters.running.store(false, Ordering::Release);
            tracing::error!("ConPTY close admission underflow");
        }
        self.shared.wake.notify_all();
    }
}

impl ConPtyCloseExecutor {
    fn start() -> io::Result<Self> {
        Self::start_config(close_capacity(), CLOSE_WORKERS, None)
    }

    fn start_config(
        capacity: usize,
        worker_count: usize,
        injected_spawn_failure: Option<usize>,
    ) -> io::Result<Self> {
        if capacity == 0 || worker_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ConPTY close executor capacity and worker count must be nonzero",
            ));
        }
        let shared = Arc::new(CloseShared {
            capacity,
            queues: Mutex::new(CloseQueues::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            counters: CloseCounters::default(),
        });
        let mut workers = Vec::with_capacity(worker_count);
        for worker_id in 0..worker_count {
            let result = if injected_spawn_failure == Some(worker_id) {
                Err(io::Error::other("injected ConPTY close worker spawn failure"))
            } else {
                let worker_shared = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name(format!("nomi-conpty-close-{worker_id}"))
                    .stack_size(CLOSE_WORKER_STACK_BYTES)
                    .spawn(move || run_close_worker(worker_shared))
            };
            match result {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    shared.shutdown.store(true, Ordering::Release);
                    shared.wake.notify_all();
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(io::Error::new(
                        error.kind(),
                        format!("start ConPTY close worker {worker_id}: {error}"),
                    ));
                }
            }
        }
        shared.counters.workers.store(worker_count, Ordering::Release);
        shared.counters.running.store(true, Ordering::Release);
        Ok(Self {
            inner: Arc::new(CloseExecutorInner {
                shared,
                workers: Mutex::new(Some(workers)),
            }),
        })
    }

    fn reserve(&self) -> io::Result<ConPtyClosePermit> {
        let shared = &self.inner.shared;
        if !shared.counters.running.load(Ordering::Acquire) {
            return Err(io::Error::other(
                "ConPTY close executor is unavailable; refusing CreatePseudoConsole",
            ));
        }
        shared
            .counters
            .admitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < shared.capacity).then_some(current + 1)
            })
            .map_err(|current| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "ConPTY close authority budget is saturated ({current}/{})",
                        shared.capacity
                    ),
                )
            })?;
        if !shared.counters.running.load(Ordering::Acquire) {
            shared.counters.admitted.fetch_sub(1, Ordering::AcqRel);
            return Err(io::Error::other(
                "ConPTY close executor stopped during admission; refusing CreatePseudoConsole",
            ));
        }
        Ok(ConPtyClosePermit {
            shared: Arc::clone(shared),
        })
    }

    fn submit(&self, job: CloseJob) -> Result<(), CloseJob> {
        let shared = &self.inner.shared;
        if !shared.counters.running.load(Ordering::Acquire) || !retain_close_job(shared) {
            return Err(job);
        }
        if !shared.counters.running.load(Ordering::Acquire) {
            release_close_job(shared);
            return Err(job);
        }
        let mut queues = shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queues.pending.push_back(job);
        let pending = queues.pending.len();
        shared.counters.pending.store(pending, Ordering::Release);
        shared
            .counters
            .peak_pending
            .fetch_max(pending, Ordering::AcqRel);
        drop(queues);
        shared.wake.notify_one();
        Ok(())
    }

    fn quarantine_unscheduled(&self, job: CloseJob, reason: Arc<str>) {
        job.completion.publish(Err(Arc::clone(&reason)));
        let shared = &self.inner.shared;
        if !retain_close_job(shared) {
            shared
                .counters
                .overflow_retained
                .fetch_add(1, Ordering::Relaxed);
            tracing::error!(%reason, "ConPTY close hard job cap reached; authority retained outside queues");
            std::mem::forget(job);
            return;
        }
        let mut queues = shared
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queues.quarantined.push(job);
        shared
            .counters
            .quarantined
            .store(queues.quarantined.len(), Ordering::Release);
    }

    #[cfg(test)]
    fn shutdown_and_join(&self) {
        let shared = &self.inner.shared;
        shared.shutdown.store(true, Ordering::Release);
        shared.wake.notify_all();
        let workers = self
            .inner
            .workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        for worker in workers {
            worker.join().expect("ConPTY close worker exits cleanly");
        }
        shared.counters.workers.store(0, Ordering::Release);
        shared.counters.running.store(false, Ordering::Release);
    }
}

fn retain_close_job(shared: &CloseShared) -> bool {
    shared
        .counters
        .retained_jobs
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < shared.capacity).then_some(current + 1)
        })
        .is_ok()
}

fn release_close_job(shared: &CloseShared) {
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
        tracing::error!("ConPTY close retained-job accounting underflow");
    }
    shared.wake.notify_all();
}

fn run_close_worker(shared: Arc<CloseShared>) {
    let _liveness = CloseWorkerLiveness {
        shared: Arc::clone(&shared),
    };
    loop {
        let job = {
            let mut queues = shared
                .queues
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(job) = queues.pending.pop_front() {
                    shared
                        .counters
                        .pending
                        .store(queues.pending.len(), Ordering::Release);
                    break job;
                }
                if shared.shutdown.load(Ordering::Acquire) {
                    return;
                }
                queues = shared
                    .wake
                    .wait(queues)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        shared.counters.active.fetch_add(1, Ordering::AcqRel);
        let outcome = catch_unwind(AssertUnwindSafe(|| job.action.run(job.handle)));
        shared.counters.active.fetch_sub(1, Ordering::AcqRel);
        match outcome {
            Ok(Ok(())) => {
                job.completion.publish(Ok(()));
                shared.counters.completed.fetch_add(1, Ordering::Relaxed);
                release_close_job(&shared);
                drop(job);
            }
            Ok(Err(error)) => {
                quarantine_close_job(
                    &shared,
                    job,
                    Arc::from(format!("ClosePseudoConsole failed: {error}")),
                );
            }
            Err(_) => {
                shared.counters.panics.fetch_add(1, Ordering::Relaxed);
                quarantine_close_job(
                    &shared,
                    job,
                    Arc::from("ClosePseudoConsole worker action panicked"),
                );
            }
        }
    }
}

fn quarantine_close_job(shared: &CloseShared, job: CloseJob, reason: Arc<str>) {
    job.completion.publish(Err(Arc::clone(&reason)));
    tracing::error!(%reason, "ConPTY close authority quarantined");
    let mut queues = shared
        .queues
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    queues.quarantined.push(job);
    shared
        .counters
        .quarantined
        .store(queues.quarantined.len(), Ordering::Release);
}

struct CloseWorkerLiveness {
    shared: Arc<CloseShared>,
}

impl Drop for CloseWorkerLiveness {
    fn drop(&mut self) {
        if self.shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        let previous = self
            .shared
            .counters
            .workers
            .fetch_sub(1, Ordering::AcqRel);
        if previous <= 1 {
            self.shared.counters.running.store(false, Ordering::Release);
        }
        self.shared.wake.notify_all();
    }
}

fn close_capacity() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .saturating_mul(CLOSE_CAPACITY_PER_CPU)
        .clamp(MIN_CLOSE_CAPACITY, MAX_CLOSE_CAPACITY)
}

fn conpty_close_executor() -> io::Result<ConPtyCloseExecutor> {
    match CLOSE_EXECUTOR.get_or_init(|| {
        ConPtyCloseExecutor::start().map_err(|error| Arc::<str>::from(error.to_string()))
    }) {
        Ok(executor) => Ok(executor.clone()),
        Err(error) => Err(io::Error::other(format!(
            "ConPTY close executor initialization failed: {error}"
        ))),
    }
}

pub(super) struct PreparedConPty {
    pub(super) control: Arc<PseudoConsoleControl>,
    pub(super) input: OwnedHandle,
    pub(super) output: OwnedHandle,
}

impl PreparedConPty {
    pub(super) fn create(
        cols: u16,
        rows: u16,
        create_pipe: impl Fn() -> io::Result<(OwnedHandle, OwnedHandle)>,
    ) -> io::Result<Self> {
        let size = checked_coord(cols, rows)?;
        // Reserve durable close ownership before CreatePseudoConsole can
        // create a physical HPCON. Saturated or quarantined cleanup debt must
        // fail before another pseudoconsole exists.
        let executor = conpty_close_executor()?;
        let authority = executor.reserve()?;
        let (input_read, input_write) = create_pipe()?;
        let (output_read, output_write) = create_pipe()?;
        let mut pseudoconsole = 0;
        let flags = 0x2 | 0x4;
        let result = unsafe {
            CreatePseudoConsole(
                size,
                input_read.as_raw(),
                output_write.as_raw(),
                flags,
                &mut pseudoconsole,
            )
        };
        hresult(result, "CreatePseudoConsole")?;

        let control = Arc::new(PseudoConsoleControl::new(
            pseudoconsole,
            executor,
            authority,
        ));
        drop(input_read);
        drop(output_write);
        Ok(Self {
            control,
            input: input_write,
            output: output_read,
        })
    }

    pub(super) fn into_parts(self) -> (Arc<PseudoConsoleControl>, OwnedHandle, OwnedHandle) {
        (self.control, self.input, self.output)
    }
}

pub(super) struct PseudoConsoleControl {
    state: Mutex<PseudoConsoleState>,
    executor: ConPtyCloseExecutor,
    close_action: CloseAction,
}

struct PseudoConsoleState {
    handle: Option<HPCON>,
    authority: Option<ConPtyClosePermit>,
    closing: Option<Arc<CloseCompletion>>,
}

impl PseudoConsoleControl {
    fn new(
        handle: HPCON,
        executor: ConPtyCloseExecutor,
        authority: ConPtyClosePermit,
    ) -> Self {
        Self {
            state: Mutex::new(PseudoConsoleState {
                handle: Some(handle),
                authority: Some(authority),
                closing: None,
            }),
            executor,
            close_action: CloseAction::System,
        }
    }

    #[cfg(test)]
    fn new_with_close(
        handle: HPCON,
        executor: ConPtyCloseExecutor,
        close: Arc<dyn Fn(HPCON) -> io::Result<()> + Send + Sync>,
    ) -> io::Result<Self> {
        let authority = executor.reserve()?;
        Ok(Self {
            state: Mutex::new(PseudoConsoleState {
                handle: Some(handle),
                authority: Some(authority),
                closing: None,
            }),
            executor,
            close_action: CloseAction::Test(close),
        })
    }

    pub(super) fn raw(&self) -> io::Result<HPCON> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("pseudoconsole state is poisoned"))?
            .handle
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "pseudoconsole is closing"))
    }

    pub(super) fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = checked_coord(cols, rows)?;
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("pseudoconsole state is poisoned"))?;
        let handle = state
            .handle
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "pseudoconsole is closed"))?;
        let result = unsafe { ResizePseudoConsole(handle, size) };
        hresult(result, "ResizePseudoConsole")
    }

    pub(super) fn begin_close(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("pseudoconsole state is poisoned"))?;
        if state.closing.is_some() {
            return Ok(());
        }
        let Some(handle) = state.handle.take() else {
            return Ok(());
        };
        let Some(authority) = state.authority.take() else {
            state.handle = Some(handle);
            return Err(io::Error::other(
                "pseudoconsole lost its reserved close authority",
            ));
        };
        let completion = Arc::new(CloseCompletion::new());
        state.closing = Some(Arc::clone(&completion));
        drop(state);
        let job = CloseJob {
            handle,
            action: self.close_action.clone(),
            completion,
            _authority: authority,
        };
        if let Err(job) = self.executor.submit(job) {
            let reason = Arc::<str>::from(
                "ConPTY close executor rejected an already-admitted close authority",
            );
            self.executor
                .quarantine_unscheduled(job, Arc::clone(&reason));
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                reason.to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn close_until(&self, deadline: Instant) -> io::Result<()> {
        self.begin_close()?;
        let closing = self
            .state
            .lock()
            .map_err(|_| io::Error::other("pseudoconsole state is poisoned"))?
            .closing
            .clone();
        let Some(closing) = closing else {
            return Ok(());
        };
        closing.wait_until(deadline)
    }
}

impl Drop for PseudoConsoleControl {
    fn drop(&mut self) {
        let state = match self.state.get_mut() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(handle) = state.handle.take() else {
            return;
        };
        let Some(authority) = state.authority.take() else {
            tracing::error!("pseudoconsole Drop lost its reserved close authority; leaking HPCON");
            return;
        };
        let completion = Arc::new(CloseCompletion::new());
        let job = CloseJob {
            handle,
            action: self.close_action.clone(),
            completion,
            _authority: authority,
        };
        if let Err(job) = self.executor.submit(job) {
            self.executor.quarantine_unscheduled(
                job,
                Arc::from("ConPTY Drop handoff failed; exact HPCON authority retained"),
            );
        }
    }
}

fn checked_coord(cols: u16, rows: u16) -> io::Result<COORD> {
    let x = i16::try_from(cols).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ConPTY columns exceed the signed 16-bit Win32 limit",
        )
    })?;
    let y = i16::try_from(rows).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ConPTY rows exceed the signed 16-bit Win32 limit",
        )
    })?;
    if x == 0 || y == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ConPTY dimensions must be non-zero",
        ));
    }
    Ok(COORD { X: x, Y: y })
}

fn hresult(result: HRESULT, operation: &'static str) -> io::Result<()> {
    if result >= 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{operation} failed with HRESULT {:#010x}",
            result as u32
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        io,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use serial_test::serial;

    use super::{ConPtyCloseExecutor, PseudoConsoleControl};

    fn wait_for(counter: &AtomicU64, expected: u64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while counter.load(Ordering::Acquire) != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(counter.load(Ordering::Acquire), expected);
    }

    fn wait_for_usize(counter: &AtomicUsize, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::Acquire) != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(counter.load(Ordering::Acquire), expected);
    }

    #[test]
    #[serial(conpty_close_executor)]
    fn close_timeout_is_off_thread_bounded_and_single_owner() {
        let executor = ConPtyCloseExecutor::start_config(4, 1, None)
            .expect("test close executor should start");
        let caller = std::thread::current().id();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_close = Arc::clone(&calls);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let release_rx = std::sync::Mutex::new(release_rx);
        let control = PseudoConsoleControl::new_with_close(
            1,
            executor.clone(),
            Arc::new(move |_handle| {
                assert_ne!(std::thread::current().id(), caller);
                calls_for_close.fetch_add(1, Ordering::SeqCst);
                let _ = entered_tx.send(());
                let _ = release_rx
                    .lock()
                    .expect("release receiver lock")
                    .recv();
                Ok(())
            }),
        )
        .expect("test close control should initialize");

        control.begin_close().expect("close should enqueue");
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close relay should enter the injected close");
        let started = Instant::now();
        let error = control
            .close_until(Instant::now() + Duration::from_millis(25))
            .expect_err("blocked close should time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(250));
        control.begin_close().expect("repeated close should be idempotent");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        release_tx.send(()).expect("close worker should be released");
        control
            .close_until(Instant::now() + Duration::from_secs(1))
            .expect("released close should complete");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        wait_for_usize(&executor.inner.shared.counters.admitted, 0);
        executor.shutdown_and_join();
    }

    #[test]
    #[serial(conpty_close_executor)]
    fn ten_thousand_drop_handoffs_use_only_the_fixed_worker_set_and_bounded_queue() {
        const JOBS: usize = 10_000;
        const WORKERS: usize = 3;
        let executor = ConPtyCloseExecutor::start_config(JOBS, WORKERS, None)
            .expect("stress close executor should start");
        let calls = Arc::new(AtomicUsize::new(0));
        let thread_ids = Arc::new(Mutex::new(HashSet::new()));
        let action = {
            let calls = Arc::clone(&calls);
            let thread_ids = Arc::clone(&thread_ids);
            Arc::new(move |_handle| {
                calls.fetch_add(1, Ordering::AcqRel);
                thread_ids
                    .lock()
                    .expect("worker-id set is not poisoned")
                    .insert(std::thread::current().id());
                Ok(())
            })
        };

        for handle in 1..=JOBS {
            let control = PseudoConsoleControl::new_with_close(
                handle as isize as _,
                executor.clone(),
                action.clone(),
            )
            .expect("every authority inside the hard cap should be admitted");
            // Cancellation/Drop uses the same durable handoff as explicit
            // close and must not create a per-handle thread.
            drop(control);
        }

        wait_for(&executor.inner.shared.counters.completed, JOBS as u64);
        wait_for_usize(&executor.inner.shared.counters.admitted, 0);
        assert_eq!(calls.load(Ordering::Acquire), JOBS);
        assert!(
            thread_ids
                .lock()
                .expect("worker-id set is not poisoned")
                .len()
                <= WORKERS
        );
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .workers
                .load(Ordering::Acquire),
            WORKERS
        );
        assert!(
            executor
                .inner
                .shared
                .counters
                .peak_pending
                .load(Ordering::Acquire)
                <= JOBS
        );
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .retained_jobs
                .load(Ordering::Acquire),
            0
        );
        executor.shutdown_and_join();
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .workers
                .load(Ordering::Acquire),
            0,
            "normal shutdown must join every fixed worker"
        );
    }

    #[test]
    #[serial(conpty_close_executor)]
    fn saturated_close_authority_rejects_n_plus_one_before_creation() {
        let executor = ConPtyCloseExecutor::start_config(2, 1, None)
            .expect("bounded close executor should start");
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(2);
        let release_rx = Arc::new(Mutex::new(release_rx));
        let calls = Arc::new(AtomicUsize::new(0));
        let action = {
            let release_rx = Arc::clone(&release_rx);
            let calls = Arc::clone(&calls);
            Arc::new(move |_handle| {
                if calls.fetch_add(1, Ordering::AcqRel) == 0 {
                    let _ = entered_tx.send(());
                }
                release_rx
                    .lock()
                    .expect("release receiver is not poisoned")
                    .recv()
                    .map_err(|error| io::Error::other(error.to_string()))?;
                Ok(())
            })
        };
        let first = PseudoConsoleControl::new_with_close(1, executor.clone(), action.clone())
            .expect("first close authority");
        let second = PseudoConsoleControl::new_with_close(2, executor.clone(), action)
            .expect("second close authority");
        first.begin_close().expect("first close enters worker");
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first close action should block in the only worker");
        second.begin_close().expect("second close queues");
        assert_eq!(
            executor
                .reserve()
                .err()
                .expect("N+1 CreatePseudoConsole authority must fail")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .pending
                .load(Ordering::Acquire),
            1
        );

        release_tx.send(()).expect("release first close");
        release_tx.send(()).expect("release second close");
        first
            .close_until(Instant::now() + Duration::from_secs(2))
            .expect("first close completes");
        second
            .close_until(Instant::now() + Duration::from_secs(2))
            .expect("second close completes");
        wait_for_usize(&executor.inner.shared.counters.admitted, 0);
        executor.shutdown_and_join();
    }

    #[test]
    fn partial_worker_start_failure_joins_started_workers_and_fails_before_admission() {
        let started = Instant::now();
        let error = ConPtyCloseExecutor::start_config(2, 2, Some(1))
            .err()
            .expect("injected second-worker failure must fail initialization");
        assert!(
            error
                .to_string()
                .contains("injected ConPTY close worker spawn failure")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    #[serial(conpty_close_executor)]
    fn close_action_panic_is_quarantined_without_losing_authority_or_worker() {
        let executor = ConPtyCloseExecutor::start_config(1, 1, None)
            .expect("panic test close executor should start");
        let control = PseudoConsoleControl::new_with_close(
            1,
            executor.clone(),
            Arc::new(move |_handle| -> io::Result<()> {
                panic!("injected ClosePseudoConsole panic")
            }),
        )
        .expect("panic test authority should be admitted");
        let error = control
            .close_until(Instant::now() + Duration::from_secs(1))
            .expect_err("panicking close action must not publish exact success");
        assert!(error.to_string().contains("panicked"));
        wait_for_usize(&executor.inner.shared.counters.quarantined, 1);
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .admitted
                .load(Ordering::Acquire),
            1
        );
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .retained_jobs
                .load(Ordering::Acquire),
            1
        );
        assert!(
            executor
                .inner
                .shared
                .counters
                .running
                .load(Ordering::Acquire)
        );
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .workers
                .load(Ordering::Acquire),
            1
        );
        executor.shutdown_and_join();
    }

    #[test]
    #[serial(conpty_close_executor)]
    fn permanent_close_failure_is_sticky_and_fails_closed_at_constant_capacity() {
        let executor = ConPtyCloseExecutor::start_config(1, 1, None)
            .expect("failure test close executor should start");
        let control = PseudoConsoleControl::new_with_close(
            1,
            executor.clone(),
            Arc::new(move |_handle| Err(io::Error::other("injected permanent close failure"))),
        )
        .expect("failure test authority should be admitted");
        let error = control
            .close_until(Instant::now() + Duration::from_secs(1))
            .expect_err("permanent close failure must not publish exact success");
        assert!(error.to_string().contains("injected permanent close failure"));
        wait_for_usize(&executor.inner.shared.counters.quarantined, 1);
        assert_eq!(
            executor
                .reserve()
                .err()
                .expect("sticky debt must fence the next physical HPCON")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            executor
                .inner
                .shared
                .counters
                .overflow_retained
                .load(Ordering::Acquire),
            0
        );
        executor.shutdown_and_join();
    }
}
