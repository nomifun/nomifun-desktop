use std::{
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, OnceLock},
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedChildDropMode {
    /// The caller still owns the hand-off decision.
    Handoff,
    /// The cleanup relay owns the value. A drop must retain it directly
    /// instead of re-entering the hand-off path.
    Retain,
}

pub struct ManagedChildProcess {
    child: Option<Child>,
    cleanup: Option<ChildProcessCleanup>,
    shutdown_complete: bool,
    drop_mode: ManagedChildDropMode,
}

const MANAGED_CLEANUP_SYNC_GRACE: Duration = Duration::from_millis(500);

/// A last-resort, process-local cleanup relay.
///
/// This is intentionally a static hand-off rather than an implicit leak:
/// ownership remains visible to the runtime and can be retried by a later
/// cleanup hand-off when an execution path becomes available again. Statics
/// are not destructed during Rust process teardown, so retaining an item here
/// does not invoke [`ManagedChildProcess::drop`].
static PENDING_MANAGED_CLEANUPS: OnceLock<Mutex<Vec<ManagedChildProcess>>> = OnceLock::new();

fn pending_managed_cleanups() -> &'static Mutex<Vec<ManagedChildProcess>> {
    PENDING_MANAGED_CLEANUPS.get_or_init(|| Mutex::new(Vec::new()))
}

impl ManagedChildProcess {
    fn from_parts(child: Child, cleanup: ChildProcessCleanup) -> Self {
        Self {
            child: Some(child),
            cleanup: Some(cleanup),
            shutdown_complete: false,
            drop_mode: ManagedChildDropMode::Handoff,
        }
    }

    fn into_cleanup_relay(mut self) -> Self {
        self.drop_mode = ManagedChildDropMode::Retain;
        self
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
            drop_mode: self.drop_mode,
        };
        match self.drop_mode {
            ManagedChildDropMode::Handoff => hand_off_managed_child_cleanup(process),
            ManagedChildDropMode::Retain => retain_pending_managed_cleanup(process),
        }
    }
}

fn hand_off_managed_child_cleanup(process: ManagedChildProcess) {
    let mut processes = take_pending_managed_cleanups();
    processes.push(process.into_cleanup_relay());

    let retained = Arc::new(Mutex::new(Some(processes)));
    let worker_retained = Arc::clone(&retained);
    let worker = move || {
        let processes = worker_retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let Some(processes) = processes else {
            return;
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(runtime) = runtime else {
            retain_after_runtime_failure(processes);
            return;
        };
        runtime.block_on(shutdown_managed_children(processes));
    };

    if std::thread::Builder::new()
        .name("nomi-managed-child-cleanup".into())
        .spawn(worker)
        .is_ok()
    {
        return;
    }

    let process = retained
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    let Some(processes) = process else {
        return;
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(shutdown_managed_children(processes));
    } else {
        retain_after_runtime_failure(processes);
    }
}

fn take_pending_managed_cleanups() -> Vec<ManagedChildProcess> {
    pending_managed_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain(..)
        .collect()
}

async fn shutdown_managed_children(processes: Vec<ManagedChildProcess>) {
    let unfinished = shutdown_children_with_bounded_retries(
        processes,
        async |process: &mut ManagedChildProcess| process.shutdown().await,
        MANAGED_CLEANUP_RETRY_DEADLINE,
        MANAGED_CLEANUP_RETRY_WAIT,
    )
    .await;
    // Exceeding the bounded window is loud but never a loss of authority: the
    // exact process handle returns to the pending relay and is retried by the
    // next cleanup hand-off, instead of spinning this worker thread forever
    // (and head-of-line-blocking every process queued behind it) on a tree
    // that never proves terminal.
    for process in unfinished {
        retain_pending_managed_cleanup(process);
    }
}

/// Per-process retry window for one cleanup pass. A stuck descendant
/// (uninterruptible I/O, an unproven platform tree) fails every attempt; the
/// deadline turns that into a retained retry instead of an infinite loop.
const MANAGED_CLEANUP_RETRY_DEADLINE: Duration = Duration::from_secs(30);
const MANAGED_CLEANUP_RETRY_WAIT: Duration = Duration::from_millis(250);

/// Drive each process's shutdown with retries bounded by `retry_deadline`,
/// returning the processes whose cleanup never proved terminal so the caller
/// can retain their exact authority for a later pass.
async fn shutdown_children_with_bounded_retries<P>(
    processes: Vec<P>,
    mut shutdown: impl AsyncFnMut(&mut P) -> io::Result<()>,
    retry_deadline: Duration,
    retry_wait: Duration,
) -> Vec<P> {
    let mut unfinished = Vec::new();
    for mut process in processes {
        let deadline = tokio::time::Instant::now() + retry_deadline;
        let mut exhausted = false;
        loop {
            match shutdown(&mut process).await {
                Ok(()) => break,
                Err(error) if tokio::time::Instant::now() >= deadline => {
                    tracing::error!(
                        %error,
                        deadline_secs = retry_deadline.as_secs(),
                        "managed child cleanup exceeded its bounded retry window"
                    );
                    exhausted = true;
                    break;
                }
                Err(error) => {
                    tracing::warn!(%error, "managed child cleanup retry is still pending");
                    tokio::time::sleep(retry_wait).await;
                }
            }
        }
        if exhausted {
            unfinished.push(process);
        }
    }
    unfinished
}

fn retain_after_runtime_failure(processes: Vec<ManagedChildProcess>) {
    for mut process in processes {
        let reaped = best_effort_synchronous_kill_and_reap(&mut process);
        retain_pending_managed_cleanup_with_status(process, reaped);
    }
}

fn retain_pending_managed_cleanup(process: ManagedChildProcess) {
    retain_pending_managed_cleanup_with_status(process, false);
}

fn retain_pending_managed_cleanup_with_status(
    mut process: ManagedChildProcess,
    direct_child_reaped: bool,
) {
    process.drop_mode = ManagedChildDropMode::Retain;
    let pid = process.child.as_ref().and_then(Child::id);
    let mut pending = pending_managed_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.push(process);
    tracing::error!(
        pid,
        direct_child_reaped,
        pending = pending.len(),
        "managed child cleanup retained for a later retry"
    );
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

    #[tokio::test(start_paused = true)]
    async fn bounded_cleanup_retries_transient_failures_until_success() {
        let unfinished = shutdown_children_with_bounded_retries(
            vec![3_u32],
            async |remaining: &mut u32| {
                if *remaining == 0 {
                    Ok(())
                } else {
                    *remaining -= 1;
                    Err(io::Error::other("transient tree-proof failure"))
                }
            },
            Duration::from_secs(30),
            Duration::from_millis(250),
        )
        .await;
        assert!(unfinished.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn bounded_cleanup_stops_spinning_on_a_never_terminal_tree() {
        // One never-terminal process must neither loop forever nor
        // head-of-line-block the processes queued behind it.
        let unfinished = shutdown_children_with_bounded_retries(
            vec!["stuck", "healthy"],
            async |process: &mut &str| {
                if *process == "stuck" {
                    Err(io::Error::other("descendant never proves terminal"))
                } else {
                    Ok(())
                }
            },
            Duration::from_secs(30),
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(unfinished, vec!["stuck"]);
    }
}
