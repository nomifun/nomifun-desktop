mod conpty;
mod handles;

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString, c_void},
    io,
    mem,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use tokio::sync::watch;
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_BROKEN_PIPE,
        ERROR_INVALID_PARAMETER, ERROR_NO_DATA, FILETIME, HANDLE, HANDLE_FLAG_INHERIT,
        WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT, SetHandleInformation,
    },
    Globalization::{CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringOrdinal},
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            IsProcessInJob,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
            JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        SystemInformation::GetWindowsDirectoryW,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
            CreateProcessW, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess,
            GetProcessId, GetProcessTimes, OpenProcess, OpenThread, PROCESS_INFORMATION,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
            PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW, ResumeThread,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, THREAD_SUSPEND_RESUME, TerminateProcess,
            WaitForSingleObject,
        },
    },
};

use self::handles::{OwnedHandle, ProcThreadAttributeList};
use self::conpty::{PreparedConPty, PseudoConsoleControl};
use super::{ExitFact, PlatformProcess, SpawnedPlatformProcess};
use super::poller::{
    LifecyclePoll, LifecyclePollJob, PlatformLifecyclePermit, platform_lifecycle_poller,
};
use crate::{
    CleanupReport, CommandSpec, ProcessError, NormalizedProcessRequest, OutputBuffer,
    OutputStream, ProcessSnapshot, ProcessState, SandboxPolicy, ShellKind, SpawnFailure,
};

const READ_BUFFER_BYTES: usize = 8 * 1024;
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const POST_EXIT_READER_DRAIN: Duration = Duration::from_secs(1);
const CONPTY_NATURAL_CLOSE_WAIT: Duration = Duration::from_millis(250);
const CONPTY_INPUT_CLOSE_GRACE: Duration = Duration::from_millis(250);
const JOB_EMPTY_POLL: Duration = Duration::from_millis(2);
const WRITE_CHUNK_BYTES: usize = 64 * 1024;
const MAX_COMMAND_LINE_UNITS: usize = 32_767;
const TERMINATED_BY_HOST_EXIT_CODE: u32 = 0xC000_013A;

/// Exact creation identity read through one live Windows process handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsProcessIdentity {
    pub pid: u32,
    pub start_time_epoch_seconds: u64,
    pub platform_start_key: u64,
    pub executable: PathBuf,
}

/// Owns an exact Windows process handle suitable for fail-closed recovery.
pub struct WindowsExactProcess {
    process: OwnedHandle,
    identity: WindowsProcessIdentity,
}

impl WindowsExactProcess {
    /// Open a process with the rights required for Job assignment,
    /// termination, exact identity inspection, and terminal waiting.
    pub fn open_for_recovery(pid: u32) -> io::Result<Self> {
        let process = open_exact_process(pid, true)?;
        let identity = process_identity_from_handle(process.as_raw())?;
        if identity.pid != pid {
            return Err(io::Error::other(format!(
                "opened process handle reports PID {}, expected {pid}",
                identity.pid
            )));
        }
        Ok(Self { process, identity })
    }

    pub fn identity(&self) -> &WindowsProcessIdentity {
        &self.identity
    }

    fn wait_until(&self, deadline: Instant) -> io::Result<()> {
        wait_handle_until(self.process.as_raw(), deadline)
    }

    fn is_terminated(&self) -> io::Result<bool> {
        handle_is_signaled(self.process.as_raw())
    }

    fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        if self.is_terminated()? {
            return Ok(());
        }
        // SAFETY: this exact process handle carries PROCESS_TERMINATE and is
        // retained until the terminal wait below completes.
        if unsafe { TerminateProcess(self.process.as_raw(), TERMINATED_BY_HOST_EXIT_CODE) } == 0 {
            let error = io::Error::last_os_error();
            if !self.is_terminated()? {
                return Err(error);
            }
        }
        self.wait_until(deadline)
    }
}

/// Inspect one PID through an exact, non-inheritable process handle.
pub fn windows_process_identity(pid: u32) -> io::Result<WindowsProcessIdentity> {
    let process = open_exact_process(pid, false)?;
    let identity = process_identity_from_handle(process.as_raw())?;
    if identity.pid != pid {
        return Err(io::Error::other(format!(
            "opened process handle reports PID {}, expected {pid}",
            identity.pid
        )));
    }
    Ok(identity)
}

/// Probe one PID for a *running* exact process.
///
/// `Ok(None)` is positive proof that no live process instance currently runs
/// under the PID: either no process object exists (`OpenProcess` rejects the
/// PID) or the object is already terminated (its handle is signaled; a
/// terminated object can no longer execute, and identity queries such as
/// `QueryFullProcessImageNameW` legitimately fail on it).
pub(crate) fn probe_running_process_identity(
    pid: u32,
) -> io::Result<Option<WindowsProcessIdentity>> {
    const ERROR_INVALID_PARAMETER: i32 = 87;
    let process = match open_exact_process(pid, false) {
        Ok(process) => process,
        Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if handle_is_signaled(process.as_raw())? {
        return Ok(None);
    }
    let identity = process_identity_from_handle(process.as_raw())?;
    if identity.pid != pid {
        return Err(io::Error::other(format!(
            "opened process handle reports PID {}, expected {pid}",
            identity.pid
        )));
    }
    Ok(Some(identity))
}

/// Inspect a Tokio child through the exact handle already owned by the child.
pub fn windows_child_process_identity(
    child: &tokio::process::Child,
) -> io::Result<WindowsProcessIdentity> {
    let raw = child
        .raw_handle()
        .ok_or_else(|| io::Error::other("child process handle is unavailable"))?;
    process_identity_from_handle(raw.cast())
}

/// A recovery-only Job assembled after caller-side durable identity validation.
///
/// The Job is deliberately unarmed while verified descendants are collected.
/// Once armed, any failure path that drops it retains kill-on-close semantics.
pub struct WindowsRecoveryJob {
    job: JobControl,
    armed: bool,
}

impl WindowsRecoveryJob {
    pub fn new_unarmed() -> io::Result<Self> {
        Ok(Self {
            job: JobControl::new(create_unarmed_process_job()?),
            armed: false,
        })
    }

    pub fn assign(&self, process: &WindowsExactProcess) -> io::Result<()> {
        let job = self
            .job
            .raw_handle()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "recovery Job is closed"))?;
        // SAFETY: both exact handles remain live for this call.
        if unsafe { AssignProcessToJobObject(job, process.process.as_raw()) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn arm_kill_on_close(&mut self) -> io::Result<()> {
        if self.armed {
            return Ok(());
        }
        let job = self
            .job
            .raw_handle()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "recovery Job is closed"))?;
        arm_process_job(job)?;
        self.armed = true;
        Ok(())
    }

    /// Terminate all members and prove every supplied exact process plus Job
    /// membership is terminal before closing the Job.
    pub fn terminate_and_wait<'a>(
        &self,
        processes: impl IntoIterator<Item = &'a WindowsExactProcess>,
        timeout: Duration,
    ) -> io::Result<()> {
        if !self.armed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery Job must be armed before termination",
            ));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
        self.job.terminate()?;
        for process in processes {
            process.wait_until(deadline)?;
        }
        self.job.wait_empty_until(deadline)?;
        self.job.close_proven_empty()
    }
}

/// Adopt and terminate one already-running exact Windows process tree.
///
/// Assigning only the root to a new Job is insufficient: Job inheritance is
/// prospective and does not pull in children that existed before recovery.
/// This routine therefore locks the root first, repeatedly snapshots and
/// opens every reachable descendant through exact process handles, validates
/// each immutable parent PID against monotonically ordered creation times,
/// and assigns all live pre-existing members before termination. A post-kill
/// pass directly terminates any descendant that raced the pre-kill adoption.
/// No absence or timeout is converted into success without exact handles.
pub(crate) fn terminate_exact_process_tree(
    root: WindowsExactProcess,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
    let mut job = WindowsRecoveryJob::new_unarmed()?;
    // Arm before the first assignment so every error path at least preserves
    // kill-on-close authority for members already adopted into this Job.
    job.arm_kill_on_close()?;
    job.assign(&root)?;
    #[cfg(test)]
    run_recovery_after_root_assign_hook();

    let mut processes = vec![root];
    loop {
        ensure_recovery_deadline(deadline)?;
        let discovered = discover_exact_descendants(&processes)?;
        if discovered.is_empty() {
            break;
        }
        for process in discovered {
            if !process.is_terminated()? {
                if let Err(assign_error) = job.assign(&process) {
                    let termination = process.terminate_until(deadline);
                    return Err(match termination {
                        Ok(()) => io::Error::new(
                            assign_error.kind(),
                            format!(
                                "assign pre-existing descendant {} to recovery Job: {assign_error}",
                                process.identity.pid
                            ),
                        ),
                        Err(termination_error) => io::Error::other(format!(
                            "assign pre-existing descendant {} to recovery Job: {assign_error}; exact fallback termination failed: {termination_error}",
                            process.identity.pid
                        )),
                    });
                }
            }
            processes.push(process);
        }
    }

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Windows recovery tree adoption exceeded its deadline",
        ));
    }
    job.terminate_and_wait(processes.iter(), remaining)?;

    // Every known parent is now pinned and terminal, so it cannot create new
    // descendants. Repeated post scans catch pre-assignment races; exact
    // handles prevent a recycled numeric PID from being signalled.
    loop {
        ensure_recovery_deadline(deadline)?;
        let discovered = discover_exact_descendants(&processes)?;
        if discovered.is_empty() {
            break;
        }
        for process in discovered {
            process.terminate_until(deadline).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "terminate raced recovery descendant {}: {error}",
                        process.identity.pid
                    ),
                )
            })?;
            processes.push(process);
        }
    }
    for process in &processes {
        process.wait_until(deadline)?;
    }
    Ok(())
}

#[cfg(test)]
type RecoveryAfterRootAssignHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
fn recovery_after_root_assign_hook() -> &'static Mutex<Option<RecoveryAfterRootAssignHook>> {
    static HOOK: OnceLock<Mutex<Option<RecoveryAfterRootAssignHook>>> = OnceLock::new();
    HOOK.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
pub(crate) fn set_recovery_after_root_assign_hook(hook: RecoveryAfterRootAssignHook) {
    *recovery_after_root_assign_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
}

#[cfg(test)]
fn run_recovery_after_root_assign_hook() {
    let hook = recovery_after_root_assign_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[derive(Clone, Copy, Debug)]
struct ProcessLineageEntry {
    pid: u32,
    parent_pid: u32,
}

fn discover_exact_descendants(
    known: &[WindowsExactProcess],
) -> io::Result<Vec<WindowsExactProcess>> {
    let snapshot = process_lineage_snapshot()?;
    let known_ids = known
        .iter()
        .map(|process| (process.identity.pid, process.identity.platform_start_key))
        .collect::<HashMap<_, _>>();
    let mut lineage_start_keys = known_ids.clone();
    let mut reachable = known_ids.keys().copied().collect::<HashSet<_>>();
    let mut ordered = Vec::new();
    loop {
        let mut progressed = false;
        for entry in &snapshot {
            if entry.pid == 0
                || reachable.contains(&entry.pid)
                || !reachable.contains(&entry.parent_pid)
            {
                continue;
            }
            reachable.insert(entry.pid);
            ordered.push(*entry);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }

    let reachable_children = ordered
        .iter()
        .map(|entry| entry.parent_pid)
        .collect::<HashSet<_>>();
    let mut discovered = Vec::new();
    for entry in ordered {
        if known_ids.contains_key(&entry.pid) {
            continue;
        }
        let process = match WindowsExactProcess::open_for_recovery(entry.pid) {
            Ok(process) => process,
            Err(error)
                if error.raw_os_error()
                    == Some(i32::try_from(ERROR_INVALID_PARAMETER).unwrap_or(87)) =>
            {
                if reachable_children.contains(&entry.pid) {
                    return Err(io::Error::other(format!(
                        "descendant {} exited before its child lineage could be locked",
                        entry.pid
                    )));
                }
                continue;
            }
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("open exact recovery descendant {}: {error}", entry.pid),
                ));
            }
        };
        let fresh_parent = process_lineage_snapshot()?
            .into_iter()
            .find(|fresh| fresh.pid == entry.pid)
            .map(|fresh| fresh.parent_pid)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "descendant {} disappeared before parent lineage revalidation",
                    entry.pid
                ))
            })?;
        if fresh_parent != entry.parent_pid {
            return Err(io::Error::other(format!(
                "descendant {} parent changed during exact-handle acquisition",
                entry.pid
            )));
        }
        let parent_start_key = lineage_start_keys
            .get(&entry.parent_pid)
            .copied()
            .ok_or_else(|| {
                io::Error::other(format!(
                    "descendant {} has no locked exact parent {}",
                    entry.pid, entry.parent_pid
                ))
            })?;
        if process.identity.platform_start_key < parent_start_key {
            return Err(io::Error::other(format!(
                "descendant {} predates exact parent {}; refusing recycled lineage",
                entry.pid, entry.parent_pid
            )));
        }
        lineage_start_keys.insert(entry.pid, process.identity.platform_start_key);
        discovered.push(process);
    }
    Ok(discovered)
}

fn process_lineage_snapshot() -> io::Result<Vec<ProcessLineageEntry>> {
    // SAFETY: the flags and process id are plain values.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    // SAFETY: a successful snapshot call returns one fresh owned handle.
    let snapshot = unsafe { OwnedHandle::from_raw(snapshot)? };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(mem::size_of::<PROCESSENTRY32W>())
            .expect("PROCESSENTRY32W fits in u32"),
        ..PROCESSENTRY32W::default()
    };
    // SAFETY: snapshot is live and entry advertises its exact writable size.
    if unsafe { Process32FirstW(snapshot.as_raw(), &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut processes = Vec::new();
    loop {
        processes.push(ProcessLineageEntry {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
        });
        // SAFETY: snapshot and writable entry remain live for iteration.
        if unsafe { Process32NextW(snapshot.as_raw(), &mut entry) } == 0 {
            break;
        }
    }
    Ok(processes)
}

fn ensure_recovery_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Windows recovery process-tree proof timed out",
        ))
    } else {
        Ok(())
    }
}

/// Owns a kill-on-close Windows Job attached to an already-started process.
///
/// The guard retains both the exact process handle and the Job handle. Dropping
/// it closes the Job and therefore terminates any remaining Job members.
pub struct WindowsProcessJob {
    process: OwnedHandle,
    job: JobControl,
}

impl WindowsProcessJob {
    /// Opens `pid` and assigns the process to a fresh kill-on-close Job.
    ///
    /// Only the rights required for Job assignment, Job termination, and exact
    /// process waiting are requested from `OpenProcess`.
    pub fn attach(pid: u32) -> io::Result<Self> {
        if pid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot attach a process Job to PID 0",
            ));
        }

        // SAFETY: OpenProcess validates the PID and returns a fresh,
        // non-inheritable exact process handle on success.
        let process = unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        // SAFETY: a non-null OpenProcess result is a fresh owned handle.
        let process = unsafe { OwnedHandle::from_raw(process) }.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("open process {pid} for Job attachment: {error}"),
            )
        })?;
        let job = JobControl::new(create_process_job()?);
        let job_handle = job
            .raw_handle()
            .ok_or_else(|| io::Error::other("new process Job closed before assignment"))?;

        // SAFETY: both handles remain live for the duration of this call and
        // the requested process rights permit Job assignment.
        if unsafe { AssignProcessToJobObject(job_handle, process.as_raw()) } == 0 {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(
                error.kind(),
                format!("assign process {pid} to process Job: {error}"),
            ));
        }

        Ok(Self { process, job })
    }

    /// Terminates all Job members and proves both exact-process exit and empty
    /// Job membership before returning.
    ///
    /// All waits share one deadline, so the total waiting time is bounded by
    /// `timeout`. A timeout leaves the guard armed; dropping it still closes
    /// the kill-on-close Job.
    pub fn terminate_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;

        self.job.terminate()?;
        wait_handle_until(self.process.as_raw(), deadline)?;
        self.job.wait_empty_until(deadline)?;
        self.job.close_proven_empty()
    }
}

pub(super) async fn spawn_pipe(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    spawn_pipe_inner(request, output, Arc::new(SystemWin32)).await
}

#[derive(Clone)]
pub(crate) struct ChildProcessCleanup {
    process: Option<Arc<ChildProcessJob>>,
}

impl ChildProcessCleanup {
    fn from_process(process: &Arc<ChildProcessJob>) -> Self {
        Self {
            process: Some(Arc::clone(process)),
        }
    }

    pub(crate) async fn wait(&self) -> io::Result<()> {
        let Some(process) = self.process.as_ref() else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "handed-off child process has no host-owned Windows Job cleanup proof",
            ));
        };
        let mut completion = process.completion.subscribe();
        let wait = async {
            loop {
                if let Some(result) = completion.borrow().clone() {
                    return result
                        .map_err(|message| io::Error::other(message.to_string()));
                }
                completion.changed().await.map_err(|_| {
                    io::Error::other(
                        "child-process Job cleanup channel closed without a result",
                    )
                })?;
            }
        };
        // The reaper has one normal proof window and, when that proof fails,
        // one kill-on-close exact-handle settlement window. Keep the public
        // wait bounded while allowing both phases to converge.
        tokio::time::timeout(
            CLEANUP_TIMEOUT + CLEANUP_TIMEOUT + Duration::from_secs(1),
            wait,
        )
            .await
            .map_err(|_| io::Error::new(
                io::ErrorKind::TimedOut,
                "child-process Job cleanup proof timed out",
            ))?
    }

    pub(crate) async fn shutdown(
        &self,
        child: &mut tokio::process::Child,
    ) -> io::Result<()> {
        let Some(process) = self.process.as_ref() else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "handed-off child process has no host-owned Windows Job cleanup authority",
            ));
        };
        request_child_process_job_shutdown(process);
        let child_result = wait_child_after_job_shutdown(process, child).await;
        let cleanup_result = self.wait().await;
        child_result?;
        cleanup_result
    }
}

pub(crate) fn spawn_child_process(
    mut command: tokio::process::Command,
    hand_off: bool,
) -> io::Result<(tokio::process::Child, ChildProcessCleanup)> {
    if hand_off {
        return command.spawn().map(|child| {
            (
                child,
                ChildProcessCleanup { process: None },
            )
        });
    }

    // Reserve cleanup authority before CreateProcess can create a physical
    // child. Quarantined Job proof debt therefore fails closed here instead
    // of multiplying another process and another reaper.
    let poller = platform_lifecycle_poller()?;
    let platform_permit = poller.reserve()?;

    command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    let job = Arc::new(JobControl::new(create_process_job()?));
    #[cfg(test)]
    let test_fault = take_test_child_spawn_fault();
    let child = command.spawn()?;
    #[cfg(test)]
    if test_fault == TestChildSpawnFault::MissingId {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::other("injected missing child id after CreateProcess"),
        ));
    }
    let Some(pid) = child.id() else {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::other("child process exited before its process Job could be registered"),
        ));
    };
    #[cfg(test)]
    if test_fault == TestChildSpawnFault::MissingRawHandle {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::other("injected missing child process handle after CreateProcess"),
        ));
    }
    let Some(process) = child.raw_handle() else {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::other("child process handle disappeared before Job assignment"),
        ));
    };
    #[cfg(test)]
    if test_fault == TestChildSpawnFault::MissingJobHandle {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::other("injected missing Job handle after CreateProcess"),
        ));
    }
    let Some(job_handle) = job.raw_handle() else {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::other("child-process Job closed before assignment"),
        ));
    };
    #[cfg(test)]
    if test_fault == TestChildSpawnFault::Assignment {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected AssignProcessToJobObject failure after CreateProcess",
            ),
        ));
    }
    // SAFETY: both handles are live for this call. The process handle remains
    // owned by tokio::process::Child and the Job is retained by `job`.
    if unsafe { AssignProcessToJobObject(job_handle, process.cast()) } == 0 {
        let assignment_error = io::Error::last_os_error();
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            false,
            io::Error::new(
                assignment_error.kind(),
                format!("assign child process to process Job: {assignment_error}"),
            ),
        ));
    }

    #[cfg(test)]
    if test_fault == TestChildSpawnFault::DuplicateHandle {
        return Err(retain_failed_windows_child_spawn(
            child,
            job,
            platform_permit,
            true,
            io::Error::other("injected child-process handle duplication failure"),
        ));
    }
    let process = match duplicate_non_inheritable(process.cast()) {
        Ok(process) => process,
        Err(error) => {
            return Err(retain_failed_windows_child_spawn(
                child,
                job,
                platform_permit,
                true,
                io::Error::new(
                    error.kind(),
                    format!("duplicate child-process handle: {error}"),
                ),
            ));
        }
    };
    let (completion, _) = watch::channel(None);
    let process = Arc::new(ChildProcessJob {
        process,
        job,
        completion,
        shutdown_requested: AtomicBool::new(false),
    });
    if let Err(error) = register_child_process_job(
        pid,
        Arc::clone(&process),
        platform_permit,
    ) {
        let _ = process.job.close_for_kill();
        let _ = unsafe { WaitForSingleObject(process.process.as_raw(), 5_000) };
        return Err(error);
    }
    if let Err(error) = resume_child_process_primary_thread(pid) {
        remove_child_process_job(pid, &process);
        let _ = process.job.close_for_kill();
        let _ = unsafe { WaitForSingleObject(process.process.as_raw(), 5_000) };
        return Err(io::Error::new(
            error.kind(),
            format!("resume child process primary thread: {error}"),
        ));
    }
    let cleanup = ChildProcessCleanup::from_process(&process);
    Ok((child, cleanup))
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TestChildSpawnFault {
    #[default]
    None,
    MissingId,
    MissingRawHandle,
    MissingJobHandle,
    Assignment,
    DuplicateHandle,
}

#[cfg(test)]
std::thread_local! {
    static TEST_CHILD_SPAWN_FAULT: std::cell::Cell<TestChildSpawnFault> =
        const { std::cell::Cell::new(TestChildSpawnFault::None) };
    static TEST_CHILD_SPAWN_COMPLETION:
        std::cell::RefCell<Option<mpsc::SyncSender<bool>>> = const {
            std::cell::RefCell::new(None)
        };
}

#[cfg(test)]
fn take_test_child_spawn_fault() -> TestChildSpawnFault {
    TEST_CHILD_SPAWN_FAULT.with(|fault| fault.replace(TestChildSpawnFault::None))
}

#[cfg(test)]
fn inject_test_child_spawn_fault(fault: TestChildSpawnFault) -> mpsc::Receiver<bool> {
    let (completion, receiver) = mpsc::sync_channel(1);
    TEST_CHILD_SPAWN_FAULT.with(|slot| {
        assert_eq!(
            slot.replace(fault),
            TestChildSpawnFault::None,
            "a child-spawn fault injection was already armed on this thread"
        );
    });
    TEST_CHILD_SPAWN_COMPLETION.with(|slot| {
        assert!(
            slot.borrow_mut().replace(completion).is_none(),
            "a child-spawn completion probe was already armed on this thread"
        );
    });
    receiver
}

#[cfg(test)]
fn take_test_child_spawn_completion() -> Option<mpsc::SyncSender<bool>> {
    TEST_CHILD_SPAWN_COMPLETION.with(|slot| slot.borrow_mut().take())
}

fn retain_failed_windows_child_spawn(
    mut child: tokio::process::Child,
    job: Arc<JobControl>,
    permit: PlatformLifecyclePermit,
    assigned_to_job: bool,
    error: io::Error,
) -> io::Error {
    let mut diagnostics = Vec::new();
    if assigned_to_job {
        if let Err(snapshot_error) = job.capture_pre_termination_members_with_retry() {
            diagnostics.push(format!(
                "capture exact suspended Job members: {snapshot_error}"
            ));
        }
        if let Err(termination_error) = job.terminate() {
            diagnostics.push(format!("terminate suspended process Job: {termination_error}"));
        }
        if let Err(snapshot_error) = job.retain_current_members() {
            diagnostics.push(format!(
                "capture post-termination suspended Job members: {snapshot_error}"
            ));
        }
    } else if let Err(kill_error) = child.start_kill()
        && kill_error.kind() != io::ErrorKind::InvalidInput
        && kill_error.kind() != io::ErrorKind::NotFound
    {
        diagnostics.push(format!("terminate exact suspended child: {kill_error}"));
    }

    let debt: Box<dyn LifecyclePollJob> = Box::new(WindowsChildSpawnFailurePoller {
        child: Some(child),
        job,
        assigned_to_job,
        started_at: Instant::now(),
        initial_error: error.to_string(),
        _permit: permit,
        #[cfg(test)]
        test_completion: take_test_child_spawn_completion(),
    });
    match platform_lifecycle_poller() {
        Ok(poller) => {
            if let Err(debt) = poller.submit(debt) {
                poller.quarantine_unscheduled(
                    debt,
                    "Windows post-CreateProcess cleanup submission failed",
                );
            }
        }
        Err(poller_error) => {
            diagnostics.push(format!("platform cleanup poller unavailable: {poller_error}"));
            std::mem::forget(debt);
        }
    }
    let suffix = if diagnostics.is_empty() {
        "exact cleanup retained by the bounded platform poller".to_owned()
    } else {
        format!(
            "exact cleanup retained by the bounded platform poller; {}",
            diagnostics.join("; ")
        )
    };
    io::Error::new(error.kind(), format!("{error}; {suffix}"))
}

struct WindowsChildSpawnFailurePoller {
    child: Option<tokio::process::Child>,
    job: Arc<JobControl>,
    assigned_to_job: bool,
    started_at: Instant,
    initial_error: String,
    _permit: PlatformLifecyclePermit,
    #[cfg(test)]
    test_completion: Option<mpsc::SyncSender<bool>>,
}

impl WindowsChildSpawnFailurePoller {
    fn poll_once(&mut self, now: Instant) -> LifecyclePoll {
        let Some(child) = self.child.as_mut() else {
            return LifecyclePoll::Quarantine {
                reason: "Windows failed-spawn direct child authority disappeared".to_owned(),
            };
        };
        match child.try_wait() {
            Ok(Some(_)) => {
                if !self.assigned_to_job {
                    return LifecyclePoll::ExactComplete;
                }
                match self
                    .job
                    .wait_empty_until(Instant::now())
                    .and_then(|_| self.job.wait_retained_members_until(Instant::now()))
                    .and_then(|_| self.job.close_proven_empty())
                {
                    Ok(()) => LifecyclePoll::ExactComplete,
                    Err(error) if now.duration_since(self.started_at) < CLEANUP_TIMEOUT => {
                        tracing::warn!(%error, "Windows failed-spawn Job proof is still pending");
                        LifecyclePoll::Pending {
                            next_poll: now + Duration::from_millis(10),
                        }
                    }
                    Err(error) => LifecyclePoll::Quarantine {
                        reason: format!(
                            "{}; suspended Job cleanup remained unproven: {error}",
                            self.initial_error
                        ),
                    },
                }
            }
            Ok(None) if now.duration_since(self.started_at) < CLEANUP_TIMEOUT => {
                let _ = child.start_kill();
                LifecyclePoll::Pending {
                    next_poll: now + Duration::from_millis(10),
                }
            }
            Ok(None) => LifecyclePoll::Quarantine {
                reason: format!(
                    "{}; exact suspended child did not reap before the bounded deadline",
                    self.initial_error
                ),
            },
            Err(error) => LifecyclePoll::Quarantine {
                reason: format!(
                    "{}; exact suspended child reap probe failed: {error}",
                    self.initial_error
                ),
            },
        }
    }

    #[cfg(test)]
    fn publish_test_completion(&mut self, outcome: &LifecyclePoll) {
        let exact = match outcome {
            LifecyclePoll::ExactComplete => true,
            LifecyclePoll::Quarantine { .. } => false,
            LifecyclePoll::Pending { .. } | LifecyclePoll::Blocking => return,
        };
        if let Some(completion) = self.test_completion.take() {
            let _ = completion.send(exact);
        }
    }
}

impl LifecyclePollJob for WindowsChildSpawnFailurePoller {
    fn poll(&mut self, now: Instant) -> LifecyclePoll {
        let outcome = self.poll_once(now);
        #[cfg(test)]
        self.publish_test_completion(&outcome);
        outcome
    }

    fn poller_failed(&mut self, _reason: &str) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        if self.assigned_to_job {
            let _ = self.job.close_for_kill();
        }
        #[cfg(test)]
        if let Some(completion) = self.test_completion.take() {
            let _ = completion.send(false);
        }
    }

    fn label(&self) -> &'static str {
        "windows-child-spawn-failure"
    }
}

pub(crate) async fn kill_process_tree(
    child: &mut tokio::process::Child,
) -> io::Result<()> {
    let Some(pid) = child.id() else {
        return child.wait().await.map(|_| ());
    };
    let Some(process) = child_process_job(pid)? else {
        child.kill().await?;
        return child.wait().await.map(|_| ());
    };

    let cleanup = ChildProcessCleanup::from_process(&process);
    request_child_process_job_shutdown(&process);
    let child_result = wait_child_after_job_shutdown(&process, child).await;
    let cleanup_result = cleanup.wait().await;
    child_result?;
    cleanup_result
}

/// Capture exact members before requesting termination, but never let a
/// fallible snapshot suppress the kill request. If TerminateJobObject itself
/// fails, retry it once before closing the kill-on-close Job so a transient
/// failure can still produce a complete post-request member snapshot.
fn request_child_process_job_shutdown(process: &ChildProcessJob) {
    process.shutdown_requested.store(true, AtomicOrdering::Release);
    if let Err(error) = process
        .job
        .capture_pre_termination_members_with_retry()
    {
        tracing::warn!(%error, "could not capture child-process Job members before termination");
    }
    match process.job.terminate() {
        Ok(()) => {
            if let Err(error) = process.job.retain_current_members() {
                tracing::warn!(%error, "could not merge child-process Job members after termination");
            }
        }
        Err(error) => {
            tracing::warn!(%error, "could not terminate child-process Job; retrying before kill-on-close");
            // A failed TerminateJobObject call does not mark termination
            // requested, so refresh the exact-member snapshot before the
            // retry. Reusing the original idempotent capture here would leave
            // members created after the first snapshot outside the retained
            // interval proof.
            if let Err(snapshot_error) = process
                .job
                .refresh_pre_termination_members_with_retry()
            {
                tracing::warn!(
                    %snapshot_error,
                    "could not capture child-process Job members before termination retry"
                );
            }
            match process.job.terminate() {
                Ok(()) => {
                    if let Err(snapshot_error) = process.job.retain_current_members() {
                        tracing::warn!(
                            %snapshot_error,
                            "could not merge child-process Job members after termination retry"
                        );
                    }
                }
                Err(retry_error) => {
                    tracing::warn!(
                        %retry_error,
                        "child-process Job termination retry failed; using kill-on-close fallback"
                    );
                    // Retain any last observable members for diagnostics and
                    // best-effort waiting, but do not mark this as a complete
                    // post-termination proof.
                    let _ = process.job.retain_current_members();
                    if let Err(close_error) = process.job.close_for_kill() {
                        tracing::warn!(
                            %close_error,
                            "could not issue child-process Job kill-on-close fallback"
                        );
                    }
                }
            }
        }
    }
}

async fn wait_child_after_job_shutdown(
    process: &ChildProcessJob,
    child: &mut tokio::process::Child,
) -> io::Result<()> {
    match tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await {
        Ok(result) => result.map(|_| ()),
        Err(_) => {
            process.job.close_for_kill()?;
            tokio::time::timeout(CLEANUP_TIMEOUT, child.wait())
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "direct child did not exit after process Job kill-on-close fallback",
                    )
                })?
                .map(|_| ())
        }
    }
}

struct ChildProcessJob {
    process: OwnedHandle,
    job: Arc<JobControl>,
    completion: watch::Sender<Option<Result<(), Arc<str>>>>,
    shutdown_requested: AtomicBool,
}

fn child_process_jobs() -> &'static Mutex<HashMap<u32, Arc<ChildProcessJob>>> {
    static JOBS: OnceLock<Mutex<HashMap<u32, Arc<ChildProcessJob>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_child_process_job(
    pid: u32,
    process: Arc<ChildProcessJob>,
    permit: PlatformLifecyclePermit,
) -> io::Result<()> {
    let registered = {
        let mut jobs = child_process_jobs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let std::collections::hash_map::Entry::Vacant(entry) = jobs.entry(pid) {
            entry.insert(Arc::clone(&process));
            true
        } else {
            false
        }
    };
    if !registered {
        let job: Box<dyn LifecyclePollJob> = Box::new(ChildProcessJobPoller {
            pid,
            process: Arc::clone(&process),
            _permit: permit,
        });
        let poller = platform_lifecycle_poller()?;
        poller.quarantine_unscheduled(
            job,
            "duplicate Windows child-process Job registry key after physical spawn",
        );
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("child-process Job already exists for PID {pid}"),
        ));
    }
    let job: Box<dyn LifecyclePollJob> = Box::new(ChildProcessJobPoller {
        pid,
        process: Arc::clone(&process),
        _permit: permit,
    });
    let poller = platform_lifecycle_poller()?;
    match poller.submit(job) {
        Ok(()) => Ok(()),
        Err(job) => {
            remove_child_process_job(pid, &process);
            poller.quarantine_unscheduled(
                job,
                "platform lifecycle poller rejected an already-admitted Windows Job",
            );
            Err(io::Error::other(
                "platform lifecycle poller rejected Windows Job registration",
            ))
        }
    }
}

fn child_process_job(pid: u32) -> io::Result<Option<Arc<ChildProcessJob>>> {
    child_process_jobs()
        .lock()
        .map_err(|_| io::Error::other("child-process Job registry is poisoned"))
        .map(|jobs| jobs.get(&pid).cloned())
}

fn remove_child_process_job(pid: u32, expected: &Arc<ChildProcessJob>) {
    let Ok(mut jobs) = child_process_jobs().lock() else {
        return;
    };
    if jobs
        .get(&pid)
        .is_some_and(|registered| Arc::ptr_eq(registered, expected))
    {
        jobs.remove(&pid);
    }
}

struct ChildProcessJobPoller {
    pid: u32,
    process: Arc<ChildProcessJob>,
    _permit: PlatformLifecyclePermit,
}

impl ChildProcessJobPoller {
    fn publish(&self, result: Result<(), Arc<str>>) {
        if self.process.completion.borrow().is_none() {
            self.process.completion.send_replace(Some(result));
        }
        remove_child_process_job(self.pid, &self.process);
    }
}

impl LifecyclePollJob for ChildProcessJobPoller {
    fn poll(&mut self, now: Instant) -> LifecyclePoll {
        match handle_is_signaled(self.process.process.as_raw()) {
            Ok(true) => LifecyclePoll::Blocking,
            Ok(false) if self.process.shutdown_requested.load(AtomicOrdering::Acquire) => {
                LifecyclePoll::Blocking
            }
            Ok(false) => LifecyclePoll::Pending {
                next_poll: now + Duration::from_millis(25),
            },
            Err(error) => {
                tracing::warn!(pid = self.pid, %error, "exact Windows child liveness probe failed; entering bounded cleanup");
                LifecyclePoll::Blocking
            }
        }
    }

    fn poll_blocking(&mut self) -> LifecyclePoll {
        match reap_child_process_job(&self.process) {
            Ok(()) => {
                self.publish(Ok(()));
                LifecyclePoll::ExactComplete
            }
            Err(error) => {
                let reason = Arc::<str>::from(error.to_string());
                self.publish(Err(Arc::clone(&reason)));
                LifecyclePoll::Quarantine {
                    reason: reason.to_string(),
                }
            }
        }
    }

    fn poller_failed(&mut self, reason: &str) {
        request_child_process_job_shutdown(&self.process);
        let _ = self.process.job.close_for_kill();
        self.publish(Err(Arc::<str>::from(reason.to_owned())));
    }

    fn label(&self) -> &'static str {
        "windows-child-process-job"
    }
}

fn reap_child_process_job(process: &ChildProcessJob) -> io::Result<()> {
    let pid = process_identity_from_handle(process.process.as_raw())
        .map(|identity| identity.pid)
        .unwrap_or(0);
    // This function runs on one of four shared workers. It must never inherit
    // the one-year live-process horizon used by a dedicated waiter: poll()
    // dispatches it only after exit/shutdown/error, and this final wait is
    // strictly bounded so four bad Jobs cannot pin the whole pool forever.
    let lifecycle_deadline = Instant::now()
        .checked_add(CLEANUP_TIMEOUT)
        .unwrap_or_else(Instant::now);
    let result = wait_handle_until(process.process.as_raw(), lifecycle_deadline).and_then(|()| {
        // Once the direct child exits, cleanup gets a short, independent
        // deadline. Reusing the one-year lifecycle horizon here used to leave
        // the reaper blocked long after every cleanup waiter had timed out.
        let cleanup_deadline = Instant::now()
            .checked_add(CLEANUP_TIMEOUT)
            .unwrap_or_else(Instant::now);
        // Snapshot first, but issue termination regardless of snapshot/query
        // failure. The retained pre-termination handles are the exact proof;
        // the post-request merge covers the narrow snapshot-to-kill interval.
        let snapshot_result = process
            .job
            .capture_pre_termination_members_with_retry();
        let termination_result = process.job.terminate();
        let supplemental_snapshot_result = process.job.retain_current_members();
        snapshot_result?;
        termination_result?;
        supplemental_snapshot_result?;
        process.job.wait_empty_until(cleanup_deadline)?;
        process
            .job
            .wait_retained_members_until(cleanup_deadline)?;
        process.job.close_proven_empty()
    });
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::warn!(
                pid,
                %error,
                "child-process Job reaper could not prove process-tree cleanup"
            );
            match settle_child_process_job_after_failure(process) {
                Ok(()) => {
                    tracing::warn!(
                        pid,
                        "child-process Job cleanup converged after kill-on-close fallback"
                    );
                    Ok(())
                }
                Err(fallback_error) => Err(io::Error::other(format!(
                    "child-process Job cleanup was not proven ({error}); fallback proof failed ({fallback_error})"
                ))),
            }
        }
    }
}

/// Last-resort convergence after the normal Job proof fails.
///
/// The Job is snapshotted once more if termination has not yet succeeded,
/// terminated again, merged with a post-request snapshot, and finally released
/// through kill-on-close. A temporary accounting/query failure is recovered
/// only when a complete pre-termination snapshot was retained and every exact
/// handle reaches the terminal state.
fn settle_child_process_job_after_failure(process: &ChildProcessJob) -> io::Result<()> {
    let mut errors = Vec::new();
    let snapshot_error = process
        .job
        .refresh_pre_termination_members_with_retry()
        .err();
    let termination_error = process.job.terminate().err();
    let supplemental_snapshot_error = process.job.retain_current_members().err();
    if let Err(error) = process.job.close_for_kill() {
        errors.push(format!("issue process Job kill-on-close fallback: {error}"));
    }

    let deadline = Instant::now()
        .checked_add(CLEANUP_TIMEOUT)
        .unwrap_or_else(Instant::now);
    if let Err(error) = wait_handle_until(process.process.as_raw(), deadline) {
        errors.push(format!("wait exact direct child after fallback: {error}"));
    }
    if let Err(error) = process.job.require_complete_member_interval_proof() {
        errors.push(format!("verify exact Job member interval after fallback: {error}"));
    } else if let Err(error) = process.job.wait_retained_members_until(deadline) {
        errors.push(format!("wait retained exact Job members after fallback: {error}"));
    }

    if errors.is_empty() {
        // Closing a kill-on-close Job and observing every exact handle reach
        // the terminal state is the proof. A preceding TerminateJobObject
        // error is therefore diagnostic only once that stronger fallback
        // proof has succeeded.
        Ok(())
    } else {
        if let Some(error) = snapshot_error {
            errors.insert(
                0,
                format!("capture exact Job members during fallback: {error}"),
            );
        }
        if let Some(error) = termination_error {
            errors.insert(0, format!("terminate process Job during fallback: {error}"));
        }
        if let Some(error) = supplemental_snapshot_error {
            errors.insert(
                0,
                format!("merge post-termination Job members during fallback: {error}"),
            );
        }
        Err(io::Error::other(errors.join("; ")))
    }
}

fn duplicate_non_inheritable(handle: HANDLE) -> io::Result<OwnedHandle> {
    // SAFETY: the current process pseudo-handle and source process handle are
    // valid; DuplicateHandle writes one fresh non-inheritable handle.
    let current = unsafe { GetCurrentProcess() };
    let mut duplicate = ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            current,
            handle,
            current,
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle returned a fresh owned handle.
    unsafe { OwnedHandle::from_raw(duplicate) }
}

fn resume_child_process_primary_thread(pid: u32) -> io::Result<()> {
    // SAFETY: the snapshot flags and process id are plain values.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    // SAFETY: a successful snapshot call returns a fresh owned handle.
    let snapshot = unsafe { OwnedHandle::from_raw(snapshot)? };
    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(mem::size_of::<THREADENTRY32>())
            .expect("THREADENTRY32 fits in u32"),
        ..THREADENTRY32::default()
    };
    // SAFETY: snapshot is live and entry is writable storage with dwSize set.
    if unsafe { Thread32First(snapshot.as_raw(), &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut only_thread = None;
    loop {
        if entry.th32OwnerProcessID == pid
            && only_thread.replace(entry.th32ThreadID).is_some()
        {
            return Err(io::Error::other(format!(
                "suspended child-process PID {pid} exposed more than one thread before resume"
            )));
        }
        // SAFETY: snapshot remains live and entry remains writable.
        if unsafe { Thread32Next(snapshot.as_raw(), &mut entry) } == 0 {
            break;
        }
    }
    let thread_id = only_thread.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("primary thread for child-process PID {pid} was not found"),
        )
    })?;
    // SAFETY: OpenThread validates the unique thread id discovered for the
    // still-suspended exact process and returns a fresh handle on success.
    let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    // SAFETY: a non-null OpenThread result is a fresh owned handle.
    let thread = unsafe { OwnedHandle::from_raw(raw)? };
    // SAFETY: CREATE_SUSPENDED prevents user code from creating any other
    // threads before this call, and the exact process handle remains live.
    let previous = unsafe { ResumeThread(thread.as_raw()) };
    match previous {
        1 => Ok(()),
        u32::MAX => Err(io::Error::last_os_error()),
        count => Err(io::Error::other(format!(
            "child-process primary thread had unexpected suspend count {count}"
        ))),
    }
}

pub(super) async fn spawn_pty(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
    cols: u16,
    rows: u16,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    spawn_inner(
        request,
        output,
        Arc::new(SystemWin32),
        SpawnTransport::Pty { cols, rows },
    )
    .await
}

    async fn spawn_pipe_inner(
        request: NormalizedProcessRequest,
        output: Arc<OutputBuffer>,
        api: Arc<dyn Win32Facade>,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    spawn_inner(request, output, api, SpawnTransport::Pipe).await
}

#[cfg(test)]
async fn spawn_pty_inner(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
    api: Arc<dyn Win32Facade>,
    cols: u16,
    rows: u16,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    spawn_inner(
        request,
        output,
        api,
        SpawnTransport::Pty { cols, rows },
    )
    .await
}

async fn spawn_inner(
    request: NormalizedProcessRequest,
    output: Arc<OutputBuffer>,
    api: Arc<dyn Win32Facade>,
    transport: SpawnTransport,
) -> Result<SpawnedPlatformProcess, ProcessError> {
    enforce_sandbox(&request)?;
    let prepared = PreparedCommand::new(&request)?;
    let setup_timeout = request
        .policy
        .deadline
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(SETUP_TIMEOUT)
        .min(SETUP_TIMEOUT);
    if setup_timeout.is_zero() {
        return Err(spawn_failed(io::Error::new(
            io::ErrorKind::TimedOut,
            "process deadline elapsed before Windows ownership setup",
        )));
    }
    let deadline = Instant::now()
        .checked_add(setup_timeout)
        .ok_or_else(|| invalid_command("Windows setup deadline overflowed"))?;
    let runtime = tokio::runtime::Handle::current();
    let platform_permit = platform_lifecycle_poller()
        .map_err(spawn_failed)?
        .reserve()
        .map_err(spawn_failed)?;
    let mut cancellation = StartCancellationGuard::new();
    let cancelled = cancellation.worker_flag();
    let mut transaction = tokio::task::spawn_blocking(move || {
        spawn_transaction(
            prepared,
            output,
            api,
            runtime,
            deadline,
            cancelled,
            transport,
            platform_permit,
        )
    });

    let owner = match tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        &mut transaction,
    )
    .await
    {
        Ok(joined) => joined
            .map_err(|error| {
                start_lost(
                    "windows_spawn_worker_failed",
                    format!("Windows spawn transaction failed to join: {error}"),
                    None,
                    CleanupReport::default(),
                )
            })??,
        Err(_) => {
            cancellation.cancel();
            match transaction.await {
                Ok(Ok(owner)) => {
                    let now = Instant::now();
                    let last_known = Some(ProcessSnapshot {
                        pid: owner.pid,
                        state: ProcessState::Lost,
                        started_at: now,
                        last_activity_at: now,
                    });
                    let cleanup_started = Instant::now();
                    let mut cleanup = CleanupReport {
                        force_kill_attempted: true,
                        ..CleanupReport::default()
                    };
                    if let Err(error) = owner.force_kill().await {
                        cleanup
                            .errors
                            .push(format!("terminate process Job after start timeout: {error}"));
                    }
                    let cleanup_deadline = Instant::now()
                        .checked_add(CLEANUP_TIMEOUT)
                        .unwrap_or_else(Instant::now);
                    match owner.wait_reaped(cleanup_deadline).await {
                        Ok(_) => cleanup.reaped = true,
                        Err(error) => cleanup.errors.push(format!(
                            "wait exact process and empty Job after start timeout: {error}"
                        )),
                    }
                    cleanup.elapsed = cleanup_started.elapsed();
                    return Err(start_lost(
                        "windows_spawn_deadline_after_resume",
                        "Windows spawn crossed its setup deadline after user code may have resumed"
                            .to_owned(),
                        last_known,
                        cleanup,
                    ));
                }
                Ok(Err(error)) => return Err(error),
                Err(error) => {
                    return Err(start_lost(
                        "windows_spawn_worker_failed",
                        format!(
                            "Windows spawn transaction failed to join after deadline cancellation: {error}"
                        ),
                        None,
                        CleanupReport {
                            errors: vec![
                                "the blocking transaction unwound before it could return a cleanup report"
                                    .to_owned(),
                            ],
                            ..CleanupReport::default()
                        },
                    ));
                }
            }
        }
    };
    cancellation.disarm();

    Ok(SpawnedPlatformProcess {
        owner: Arc::new(owner),
    })
}

#[derive(Clone, Copy)]
enum SpawnTransport {
    Pipe,
    Pty { cols: u16, rows: u16 },
}

fn spawn_transaction(
    mut prepared: PreparedCommand,
    output: Arc<OutputBuffer>,
    api: Arc<dyn Win32Facade>,
    runtime: tokio::runtime::Handle,
    deadline: Instant,
    cancellation: Arc<StartCancellation>,
    transport: SpawnTransport,
    platform_permit: PlatformLifecyclePermit,
) -> Result<WindowsOwner, ProcessError> {
    ensure_setup_active(deadline, &cancellation).map_err(spawn_failed)?;

    let io = PreparedIo::new(transport).map_err(spawn_failed)?;

    ensure_setup_active(deadline, &cancellation).map_err(spawn_failed)?;
    let job = create_process_job().map_err(spawn_failed)?;
    let job = Arc::new(JobControl::new(job));
    let mut transaction = SpawnTransaction::new(Arc::clone(&job), io, platform_permit);

    let mut attributes = match ProcThreadAttributeList::new_one() {
        Ok(attributes) => attributes,
        Err(error) => return Err(transaction.into_failure(error, deadline)),
    };
    if let Err(error) = transaction.configure_attributes(&mut attributes) {
        return Err(transaction.into_failure(error, deadline));
    }

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb =
        u32::try_from(mem::size_of::<STARTUPINFOEXW>()).expect("STARTUPINFOEXW fits in u32");
    transaction.configure_startup(&mut startup);
    startup.lpAttributeList = attributes.as_mut_ptr();

    if let Err(error) = ensure_setup_active(deadline, &cancellation) {
        return Err(transaction.into_failure(error, deadline));
    }

    let mut process_information = PROCESS_INFORMATION::default();
    let flags = CREATE_SUSPENDED
        | CREATE_UNICODE_ENVIRONMENT
        | EXTENDED_STARTUPINFO_PRESENT
        | transaction.extra_creation_flags();
    // SAFETY: every pointer refers to live, NUL-terminated backing storage for
    // the duration of the call. The mutable command-line buffer is exclusively
    // owned by this transaction. The handle list contains exactly the three
    // inheritable child stdio handles.
    let created = unsafe {
        CreateProcessW(
            prepared.application.as_ptr(),
            prepared.command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            transaction.inherit_handles(),
            flags,
            prepared.environment.as_ptr().cast::<c_void>(),
            prepared.cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process_information,
        )
    };
    if created == 0 {
        return Err(transaction.into_failure(io::Error::last_os_error(), deadline));
    }

    let process = match unsafe { OwnedHandle::from_raw(process_information.hProcess) } {
        Ok(process) => Arc::new(process),
        Err(error) => {
            if !process_information.hThread.is_null() {
                // SAFETY: CreateProcessW returned this fresh thread handle, but
                // the process handle was invalid so there is no safe transaction
                // object to adopt it into.
                let _ = unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(process_information.hThread)
                };
            }
            return Err(transaction.into_failure(error, deadline));
        }
    };
    let thread = match unsafe { OwnedHandle::from_raw(process_information.hThread) } {
        Ok(thread) => thread,
        Err(error) => {
            transaction.process = Some(process);
            transaction.phase = SpawnPhase::Suspended;
            return Err(transaction.into_failure(error, deadline));
        }
    };
    transaction.pid = Some(process_information.dwProcessId);
    transaction.process = Some(process);
    transaction.thread = Some(thread);
    transaction.phase = SpawnPhase::Suspended;
    #[cfg(test)]
    if let Err(error) = api.process_created(
        process_information.dwProcessId,
        transaction
            .process
            .as_ref()
            .expect("created process is owned")
            .as_raw(),
        transaction
            .thread
            .as_ref()
            .expect("created primary thread is owned")
            .as_raw(),
    ) {
        return Err(transaction.into_failure(error, deadline));
    }

    if let Err(error) = ensure_setup_active(deadline, &cancellation) {
        return Err(transaction.into_failure(error, deadline));
    }
    let job_handle = match job.raw_handle() {
        Some(handle) => handle,
        None => {
            return Err(transaction.into_failure(
                io::Error::other("process Job closed before assignment"),
                deadline,
            ));
        }
    };
    if let Err(error) = api.assign_process_to_job(
        job_handle,
        transaction
            .process
            .as_ref()
            .expect("created process is owned")
            .as_raw(),
    ) {
        return Err(transaction.into_failure(error, deadline));
    }
    if let Err(error) = ensure_setup_active(deadline, &cancellation) {
        return Err(transaction.into_failure(error, deadline));
    }
    let previous_suspend_count = match cancellation.resume_thread_if_active(
        deadline,
        api.as_ref(),
        transaction
            .thread
            .as_ref()
            .expect("created primary thread is owned")
            .as_raw(),
    ) {
        Ok(count) => count,
        Err(error) => return Err(transaction.into_failure(error, deadline)),
    };
    match previous_suspend_count {
        1 => transaction.phase = SpawnPhase::Resumed,
        0 => {
            transaction.phase = SpawnPhase::Resumed;
            return Err(transaction.into_failure(
                io::Error::other(
                    "ResumeThread reported an already-running primary thread",
                ),
                deadline,
            ));
        }
        count => {
            return Err(transaction.into_failure(
                io::Error::other(format!(
                    "ResumeThread left an unexpected suspend count: {count}"
                )),
                deadline,
            ));
        }
    }

    transaction.close_child_only_handles();
    transaction.thread.take();

    if let Err(error) = ensure_setup_active(deadline, &cancellation) {
        return Err(transaction.into_failure(error, deadline));
    }

    let (stdin, readers, pseudoconsole) = transaction.take_runtime_io(&runtime, output);
    let owner_pseudoconsole = transaction.pseudoconsole_control();
    let pty_input_close_not_before = owner_pseudoconsole.as_ref().map(|_| {
        Instant::now()
            .checked_add(CONPTY_INPUT_CLOSE_GRACE)
            .unwrap_or_else(Instant::now)
    });
    let (completion_sender, completion_receiver) = watch::channel(LifecycleCompletion::Running);
    let lifecycle_process = Arc::clone(
        transaction
            .process
            .as_ref()
            .expect("created process remains owned"),
    );
    let lifecycle_job = Arc::clone(&job);
    let permit = transaction
        .platform_permit
        .take()
        .expect("Windows spawn retained platform lifecycle admission");
    let wrapped: Box<dyn LifecyclePollJob> = Box::new(WindowsLifecyclePollerJob {
        process: Arc::clone(&lifecycle_process),
        job: Arc::clone(&lifecycle_job),
        readers: Some(readers),
        pseudoconsole: pseudoconsole.clone(),
        completion: completion_sender,
        _permit: permit,
    });
    let poller = platform_lifecycle_poller()
        .expect("Windows lifecycle poller was initialized before physical spawn");
    if let Err(job) = poller.submit(wrapped) {
        poller.quarantine_unscheduled(
            job,
            "Windows lifecycle poller rejected an already-admitted process",
        );
        transaction.disarmed = true;
        return Err(start_lost(
            "windows_lifecycle_poller_rejected",
            "Windows process cleanup authority was quarantined before publication".to_owned(),
            transaction.pid.map(|pid| ProcessSnapshot {
                pid,
                state: ProcessState::Lost,
                started_at: Instant::now(),
                last_activity_at: Instant::now(),
            }),
            CleanupReport {
                force_kill_attempted: true,
                reaped: false,
                errors: vec![
                    "platform poller retained exact Windows Job/process authority".to_owned(),
                ],
                ..CleanupReport::default()
            },
        ));
    }

    let pid = transaction.pid.expect("created process has a PID");
    let process = transaction
        .process
        .take()
        .expect("created process is transferred to the owner");
    transaction.disarmed = true;
    Ok(WindowsOwner {
        pid,
        process,
        job,
        stdin: Arc::new(tokio::sync::Mutex::new(Some(stdin))),
        pseudoconsole: owner_pseudoconsole,
        pty_input_close_not_before,
        completion: completion_receiver,
    })
}

/// One live supervisor process monitored by the process-wide fixed poller.
/// The short `poll` path never waits; only terminal cleanup reaches one of the
/// four bounded blocking workers.
struct WindowsLifecyclePollerJob {
    process: Arc<OwnedHandle>,
    job: Arc<JobControl>,
    readers: Option<Vec<ReaderCompletion>>,
    pseudoconsole: Option<Arc<PseudoConsoleControl>>,
    completion: watch::Sender<LifecycleCompletion>,
    _permit: PlatformLifecyclePermit,
}

impl LifecyclePollJob for WindowsLifecyclePollerJob {
    fn poll(&mut self, now: Instant) -> LifecyclePoll {
        match handle_is_signaled(self.process.as_raw()) {
            Ok(true) | Err(_) => LifecyclePoll::Blocking,
            Ok(false) => LifecyclePoll::Pending {
                next_poll: now + Duration::from_millis(25),
            },
        }
    }

    fn poll_blocking(&mut self) -> LifecyclePoll {
        let readers = self.readers.take().unwrap_or_default();
        let failure_pseudoconsole = self.pseudoconsole.clone();
        let lifecycle_deadline = Instant::now()
            .checked_add(CLEANUP_TIMEOUT)
            .unwrap_or_else(Instant::now);
        match run_lifecycle(
            Arc::clone(&self.process),
            Arc::clone(&self.job),
            readers,
            self.pseudoconsole.take(),
            lifecycle_deadline,
        ) {
            Ok(fact) => {
                self.completion
                    .send_replace(LifecycleCompletion::Reaped(fact));
                LifecyclePoll::ExactComplete
            }
            Err(error) => {
                let kind = error.kind();
                let mut message =
                    lifecycle_failure_message(error, &self.process, &self.job);
                if let Some(pseudoconsole) = failure_pseudoconsole {
                    let close_deadline = Instant::now()
                        .checked_add(conpty::CLOSE_TIMEOUT)
                        .unwrap_or_else(Instant::now);
                    if let Err(error) = pseudoconsole.close_until(close_deadline) {
                        message.push_str(&format!(
                            "; close pseudoconsole after lifecycle failure: {error}"
                        ));
                    }
                }
                self.completion.send_replace(LifecycleCompletion::Failed {
                    kind,
                    message: Arc::from(message.clone()),
                });
                if self.job.is_proven_empty_and_closed() {
                    LifecyclePoll::ExactComplete
                } else {
                    LifecyclePoll::Quarantine { reason: message }
                }
            }
        }
    }

    fn poller_failed(&mut self, reason: &str) {
        let _ = self.job.terminate();
        let _ = self.job.close_for_kill();
        if let Some(pseudoconsole) = self.pseudoconsole.as_ref() {
            let _ = pseudoconsole.begin_close();
        }
        self.completion.send_replace(LifecycleCompletion::Failed {
            kind: io::ErrorKind::Other,
            message: Arc::from(reason.to_owned()),
        });
    }

    fn label(&self) -> &'static str {
        "windows-process-lifecycle"
    }
}

/// Sticky proof debt created only when a pre-publication spawn cleanup could
/// not be proven. The Job/process handles and the admission lease are retained
/// together, so repeated failures eventually fence new physical spawns rather
/// than growing without bound.
struct WindowsFailedSpawnDebt {
    process: Option<Arc<OwnedHandle>>,
    job: Arc<JobControl>,
    _permit: PlatformLifecyclePermit,
}

impl LifecyclePollJob for WindowsFailedSpawnDebt {
    fn poll(&mut self, _now: Instant) -> LifecyclePoll {
        LifecyclePoll::Quarantine {
            reason: "Windows pre-publication process cleanup remains unproven".to_owned(),
        }
    }

    fn poller_failed(&mut self, _reason: &str) {
        let _ = self.job.terminate();
        let _ = self.job.close_for_kill();
        // Retain the exact process handle even after kill-on-close; absence is
        // intentionally not claimed without the missing Job interval proof.
        let _ = self.process.as_ref();
    }

    fn label(&self) -> &'static str {
        "windows-failed-spawn-debt"
    }
}

trait Win32Facade: Send + Sync {
    #[cfg(test)]
    fn process_created(&self, _pid: u32, _process: HANDLE, _thread: HANDLE) -> io::Result<()> {
        Ok(())
    }

    fn assign_process_to_job(&self, job: HANDLE, process: HANDLE) -> io::Result<()>;
    fn resume_thread(&self, thread: HANDLE) -> io::Result<u32>;
}

struct SystemWin32;

impl Win32Facade for SystemWin32 {
    fn assign_process_to_job(&self, job: HANDLE, process: HANDLE) -> io::Result<()> {
        // SAFETY: both handles are live kernel handles owned by the spawn
        // transaction, and the process is still suspended.
        if unsafe { AssignProcessToJobObject(job, process) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn resume_thread(&self, thread: HANDLE) -> io::Result<u32> {
        // SAFETY: thread is the live primary thread returned by CreateProcessW.
        let previous = unsafe { ResumeThread(thread) };
        if previous == u32::MAX {
            Err(io::Error::last_os_error())
        } else {
            Ok(previous)
        }
    }
}

struct StartCancellationGuard {
    state: Arc<StartCancellation>,
    armed: bool,
}

impl StartCancellationGuard {
    fn new() -> Self {
        Self {
            state: Arc::new(StartCancellation {
                cancelled: AtomicBool::new(false),
                resume_gate: Mutex::new(()),
            }),
            armed: true,
        }
    }

    fn worker_flag(&self) -> Arc<StartCancellation> {
        Arc::clone(&self.state)
    }

    fn cancel(&self) {
        self.state.cancel();
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

struct StartCancellation {
    cancelled: AtomicBool,
    resume_gate: Mutex<()>,
}

impl StartCancellation {
    fn cancel(&self) {
        let _gate = match self.resume_gate.lock() {
            Ok(gate) => gate,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.cancelled.store(true, AtomicOrdering::Release);
    }

    fn resume_thread_if_active(
        &self,
        deadline: Instant,
        api: &dyn Win32Facade,
        thread: HANDLE,
    ) -> io::Result<u32> {
        let _gate = self
            .resume_gate
            .lock()
            .map_err(|_| io::Error::other("Windows resume cancellation gate is poisoned"))?;
        ensure_setup_active(deadline, self)?;
        api.resume_thread(thread)
    }
}

impl Drop for StartCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancel();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpawnPhase {
    Preparing,
    Suspended,
    Resumed,
}

struct SpawnTransaction {
    pid: Option<u32>,
    process: Option<Arc<OwnedHandle>>,
    thread: Option<OwnedHandle>,
    job: Arc<JobControl>,
    io: Option<PreparedIo>,
    phase: SpawnPhase,
    platform_permit: Option<PlatformLifecyclePermit>,
    disarmed: bool,
}

impl SpawnTransaction {
    fn new(
        job: Arc<JobControl>,
        io: PreparedIo,
        platform_permit: PlatformLifecyclePermit,
    ) -> Self {
        Self {
            pid: None,
            process: None,
            thread: None,
            job,
            io: Some(io),
            phase: SpawnPhase::Preparing,
            platform_permit: Some(platform_permit),
            disarmed: false,
        }
    }

    fn configure_attributes(
        &mut self,
        attributes: &mut ProcThreadAttributeList,
    ) -> io::Result<()> {
        self.io
            .as_ref()
            .expect("transport I/O is owned")
            .configure_attributes(attributes)
    }

    fn configure_startup(&self, startup: &mut STARTUPINFOEXW) {
        self.io
            .as_ref()
            .expect("transport I/O is owned")
            .configure_startup(startup);
    }

    fn inherit_handles(&self) -> i32 {
        self.io
            .as_ref()
            .expect("transport I/O is owned")
            .inherit_handles()
    }

    fn extra_creation_flags(&self) -> u32 {
        self.io
            .as_ref()
            .expect("transport I/O is owned")
            .extra_creation_flags()
    }

    fn close_child_only_handles(&mut self) {
        self.io
            .as_mut()
            .expect("transport I/O is owned")
            .close_child_only_handles();
    }

    fn take_runtime_io(
        &mut self,
        runtime: &tokio::runtime::Handle,
        output: Arc<OutputBuffer>,
    ) -> (
        OwnedHandle,
        Vec<ReaderCompletion>,
        Option<Arc<PseudoConsoleControl>>,
    ) {
        self.io
            .as_mut()
            .expect("transport I/O is owned")
            .take_runtime_io(runtime, output)
    }

    fn pseudoconsole_control(&self) -> Option<Arc<PseudoConsoleControl>> {
        self.io
            .as_ref()
            .and_then(PreparedIo::pseudoconsole_control)
    }

    fn into_failure(mut self, error: io::Error, setup_deadline: Instant) -> ProcessError {
        let phase = self.phase;
        let last_known = self.pid.map(|pid| ProcessSnapshot {
            pid,
            state: ProcessState::Lost,
            started_at: Instant::now(),
            last_activity_at: Instant::now(),
        });
        let cleanup_deadline = setup_deadline.max(
            Instant::now()
                .checked_add(CLEANUP_TIMEOUT)
                .unwrap_or(setup_deadline),
        );
        let cleanup = self.cleanup(cleanup_deadline);
        if !cleanup.reaped
            && let Some(permit) = self.platform_permit.take()
        {
            let debt: Box<dyn LifecyclePollJob> = Box::new(WindowsFailedSpawnDebt {
                process: self.process.take(),
                job: Arc::clone(&self.job),
                _permit: permit,
            });
            if let Ok(poller) = platform_lifecycle_poller() {
                poller.quarantine_unscheduled(
                    debt,
                    "Windows spawn cleanup ended without exact process-tree proof",
                );
            } else {
                std::mem::forget(debt);
            }
        }
        self.disarmed = true;

        if phase != SpawnPhase::Resumed && cleanup.reaped && cleanup.errors.is_empty() {
            spawn_failed(error)
        } else {
            start_lost(
                if phase == SpawnPhase::Resumed {
                    "windows_post_resume_failure"
                } else {
                    "windows_pre_resume_cleanup_unproven"
                },
                error.to_string(),
                last_known,
                cleanup,
            )
        }
    }

    fn cleanup(&mut self, deadline: Instant) -> CleanupReport {
        let mut cleanup = CleanupReport {
            force_kill_attempted: self.process.is_some(),
            ..CleanupReport::default()
        };
        let started = Instant::now();
        self.io
            .as_mut()
            .map(PreparedIo::close_input_and_child_handles);
        self.thread.take();

        if let Some(process) = &self.process {
            if self.phase == SpawnPhase::Resumed {
                if let Err(error) = self.job.terminate() {
                    cleanup
                        .errors
                        .push(format!("terminate process Job: {error}"));
                }
            } else {
                // SAFETY: process is the exact suspended process created by
                // this transaction. No user instruction has run.
                if unsafe { TerminateProcess(process.as_raw(), TERMINATED_BY_HOST_EXIT_CODE) } == 0 {
                    let error = io::Error::last_os_error();
                    if !handle_is_signaled(process.as_raw()).unwrap_or(false) {
                        cleanup
                            .errors
                            .push(format!("terminate suspended process: {error}"));
                    }
                }
            }

            match wait_handle_until(process.as_raw(), deadline) {
                Ok(()) => {
                    cleanup.reaped = true;
                    if let Err(error) = self.job.wait_empty_until(deadline) {
                        cleanup
                            .errors
                            .push(format!("prove process Job empty: {error}"));
                    } else if let Err(error) = self.job.close_proven_empty() {
                        cleanup
                            .errors
                            .push(format!("close empty process Job: {error}"));
                    }
                }
                Err(error) => cleanup
                    .errors
                    .push(format!("wait exact process handle: {error}")),
            }
        } else {
            if let Err(error) = self.job.wait_empty_until(deadline) {
                cleanup
                    .errors
                    .push(format!("prove uncommitted process Job empty: {error}"));
            } else if let Err(error) = self.job.close_proven_empty() {
                cleanup
                    .errors
                    .push(format!("close uncommitted process Job: {error}"));
            } else {
                cleanup.reaped = true;
            }
        }
        if !cleanup.errors.is_empty() {
            let _ = self.job.close_for_kill();
        }
        if let Some(io) = self.io.as_mut()
            && let Err(error) = io.finish_cleanup(deadline)
        {
            cleanup
                .errors
                .push(format!("close Windows transport: {error}"));
        }
        self.io.take();
        cleanup.elapsed = started.elapsed();
        cleanup
    }
}

impl Drop for SpawnTransaction {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        let deadline = Instant::now()
            .checked_add(CLEANUP_TIMEOUT)
            .unwrap_or_else(Instant::now);
        let _ = self.cleanup(deadline);
    }
}

struct PipePair {
    read: OwnedHandle,
    write: OwnedHandle,
}

enum PreparedIo {
    Pipe {
        parent_stdin: Option<OwnedHandle>,
        child_stdin: Option<OwnedHandle>,
        parent_stdout: Option<OwnedHandle>,
        child_stdout: Option<OwnedHandle>,
        parent_stderr: Option<OwnedHandle>,
        child_stderr: Option<OwnedHandle>,
    },
    Pty {
        input: Option<OwnedHandle>,
        output: Option<OwnedHandle>,
        control: Arc<PseudoConsoleControl>,
    },
}

impl PreparedIo {
    fn new(transport: SpawnTransport) -> io::Result<Self> {
        match transport {
            SpawnTransport::Pipe => {
                let stdin = create_inheritable_pipe()?;
                let stdout = create_inheritable_pipe()?;
                let stderr = create_inheritable_pipe()?;
                clear_inheritance(stdin.write.as_raw())?;
                clear_inheritance(stdout.read.as_raw())?;
                clear_inheritance(stderr.read.as_raw())?;
                Ok(Self::Pipe {
                    parent_stdin: Some(stdin.write),
                    child_stdin: Some(stdin.read),
                    parent_stdout: Some(stdout.read),
                    child_stdout: Some(stdout.write),
                    parent_stderr: Some(stderr.read),
                    child_stderr: Some(stderr.write),
                })
            }
            SpawnTransport::Pty { cols, rows } => {
                let prepared = PreparedConPty::create(cols, rows, || {
                    let pair = create_inheritable_pipe()?;
                    Ok((pair.read, pair.write))
                })?;
                clear_inheritance(prepared.input.as_raw())?;
                clear_inheritance(prepared.output.as_raw())?;
                let (control, input, output) = prepared.into_parts();
                Ok(Self::Pty {
                    input: Some(input),
                    output: Some(output),
                    control,
                })
            }
        }
    }

    fn configure_attributes(
        &self,
        attributes: &mut ProcThreadAttributeList,
    ) -> io::Result<()> {
        match self {
            Self::Pipe {
                child_stdin,
                child_stdout,
                child_stderr,
                ..
            } => attributes.set_handle_list(&[
                child_stdin.as_ref().expect("child stdin is owned").as_raw(),
                child_stdout
                    .as_ref()
                    .expect("child stdout is owned")
                    .as_raw(),
                child_stderr
                    .as_ref()
                    .expect("child stderr is owned")
                    .as_raw(),
            ]),
            Self::Pty { control, .. } => attributes.set_pseudoconsole(control.raw()?),
        }
    }

    fn configure_startup(&self, startup: &mut STARTUPINFOEXW) {
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        match self {
            Self::Pipe {
                child_stdin,
                child_stdout,
                child_stderr,
                ..
            } => {
                startup.StartupInfo.hStdInput =
                    child_stdin.as_ref().expect("child stdin is owned").as_raw();
                startup.StartupInfo.hStdOutput = child_stdout
                    .as_ref()
                    .expect("child stdout is owned")
                    .as_raw();
                startup.StartupInfo.hStdError = child_stderr
                    .as_ref()
                    .expect("child stderr is owned")
                    .as_raw();
            }
            Self::Pty { .. } => {
                startup.StartupInfo.hStdInput = -1isize as HANDLE;
                startup.StartupInfo.hStdOutput = -1isize as HANDLE;
                startup.StartupInfo.hStdError = -1isize as HANDLE;
            }
        }
    }

    const fn inherit_handles(&self) -> i32 {
        match self {
            Self::Pipe { .. } => 1,
            Self::Pty { .. } => 0,
        }
    }

    const fn extra_creation_flags(&self) -> u32 {
        match self {
            Self::Pipe { .. } => CREATE_NO_WINDOW,
            Self::Pty { .. } => 0,
        }
    }

    fn close_child_only_handles(&mut self) {
        if let Self::Pipe {
            child_stdin,
            child_stdout,
            child_stderr,
            ..
        } = self
        {
            child_stdin.take();
            child_stdout.take();
            child_stderr.take();
        }
    }

    fn take_runtime_io(
        &mut self,
        runtime: &tokio::runtime::Handle,
        output: Arc<OutputBuffer>,
    ) -> (
        OwnedHandle,
        Vec<ReaderCompletion>,
        Option<Arc<PseudoConsoleControl>>,
    ) {
        match self {
            Self::Pipe {
                parent_stdin,
                parent_stdout,
                parent_stderr,
                ..
            } => {
                let stdin = parent_stdin.take().expect("parent stdin is owned");
                let stdout = parent_stdout.take().expect("parent stdout is owned");
                let stderr = parent_stderr.take().expect("parent stderr is owned");
                (
                    stdin,
                    vec![
                        start_reader(
                            runtime,
                            stdout,
                            OutputStream::Stdout,
                            Arc::clone(&output),
                        ),
                        start_reader(runtime, stderr, OutputStream::Stderr, output),
                    ],
                    None,
                )
            }
            Self::Pty {
                input,
                output: pty_output,
                control,
            } => {
                let input = input.take().expect("ConPTY input is owned");
                let output_handle = pty_output.take().expect("ConPTY output is owned");
                (
                    input,
                    vec![start_reader(runtime, output_handle, OutputStream::Pty, output)],
                    Some(Arc::clone(control)),
                )
            }
        }
    }

    fn pseudoconsole_control(&self) -> Option<Arc<PseudoConsoleControl>> {
        match self {
            Self::Pipe { .. } => None,
            Self::Pty { control, .. } => Some(Arc::clone(control)),
        }
    }

    fn close_input_and_child_handles(&mut self) {
        match self {
            Self::Pipe {
                parent_stdin,
                child_stdin,
                child_stdout,
                child_stderr,
                ..
            } => {
                parent_stdin.take();
                child_stdin.take();
                child_stdout.take();
                child_stderr.take();
            }
            Self::Pty { input, .. } => {
                input.take();
            }
        }
    }

    fn finish_cleanup(&mut self, deadline: Instant) -> io::Result<()> {
        match self {
            Self::Pipe { .. } => Ok(()),
            Self::Pty {
                input,
                output,
                control,
            } => {
                input.take();
                let close_deadline = deadline.min(
                    Instant::now()
                        .checked_add(conpty::CLOSE_TIMEOUT)
                        .unwrap_or(deadline),
                );
                let result = control.close_until(close_deadline);
                output.take();
                result
            }
        }
    }
}

fn create_inheritable_pipe() -> io::Result<PipePair> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>())
            .expect("SECURITY_ATTRIBUTES fits in u32"),
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    // SAFETY: output pointers and SECURITY_ATTRIBUTES are valid for the call.
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreatePipe succeeded and returned two fresh owned handles.
    let read = unsafe { OwnedHandle::from_raw(read)? };
    // SAFETY: CreatePipe succeeded and returned a second fresh owned handle.
    let write = unsafe { OwnedHandle::from_raw(write)? };
    Ok(PipePair { read, write })
}

fn clear_inheritance(handle: HANDLE) -> io::Result<()> {
    // SAFETY: handle is a live pipe handle owned by the transaction.
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn create_process_job() -> io::Result<OwnedHandle> {
    let job = create_unarmed_process_job()?;
    arm_process_job(job.as_raw())?;
    Ok(job)
}

fn create_unarmed_process_job() -> io::Result<OwnedHandle> {
    // SAFETY: null security/name pointers request a non-inheritable anonymous Job.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    // SAFETY: a non-null result is a fresh Job handle.
    unsafe { OwnedHandle::from_raw(raw) }
}

fn arm_process_job(job: HANDLE) -> io::Result<()> {
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: job and limits remain valid for the duration of the call.
    if unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
            u32::try_from(mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .expect("JOBOBJECT_EXTENDED_LIMIT_INFORMATION fits in u32"),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn open_exact_process(pid: u32, recovery: bool) -> io::Result<OwnedHandle> {
    if pid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot open exact process PID 0",
        ));
    }
    let mut access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
    if recovery {
        access |= PROCESS_SET_QUOTA | PROCESS_TERMINATE;
    }
    // SAFETY: OpenProcess validates the PID and returns a fresh,
    // non-inheritable handle on success.
    let raw = unsafe { OpenProcess(access, 0, pid) };
    // SAFETY: a non-null OpenProcess result is a fresh owned handle.
    unsafe { OwnedHandle::from_raw(raw) }
}

fn process_identity_from_handle(handle: HANDLE) -> io::Result<WindowsProcessIdentity> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: every FILETIME pointer is writable and the exact process handle
    // remains live for the complete query.
    if unsafe {
        GetProcessTimes(
            handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let platform_start_key =
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    const WINDOWS_TO_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
    if platform_start_key <= WINDOWS_TO_UNIX_EPOCH_100NS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process creation FILETIME is invalid",
        ));
    }
    let start_time_epoch_seconds =
        (platform_start_key - WINDOWS_TO_UNIX_EPOCH_100NS) / 10_000_000;

    let mut image = vec![0_u16; 32_768];
    let mut image_len = image.len() as u32;
    // SAFETY: image is writable for image_len UTF-16 units and the exact
    // process handle remains live.
    if unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            image.as_mut_ptr(),
            &mut image_len,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if image_len == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process executable query returned an empty path",
        ));
    }
    let executable = PathBuf::from(OsString::from_wide(&image[..image_len as usize]));
    // SAFETY: handle remains live and identifies one exact process.
    let pid = unsafe { GetProcessId(handle) };
    if pid == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(WindowsProcessIdentity {
        pid,
        start_time_epoch_seconds,
        platform_start_key,
        executable,
    })
}

struct JobControl {
    state: Mutex<JobState>,
    #[cfg(test)]
    test_member_snapshot_failures: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    test_supplemental_snapshot_failures: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    test_wait_empty_failures: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    test_terminate_failures: std::sync::atomic::AtomicUsize,
}

struct JobState {
    handle: Option<OwnedHandle>,
    termination_requested: bool,
    empty_proven: bool,
    retained_member_handles: Vec<OwnedHandle>,
    pre_termination_snapshot_complete: bool,
    post_termination_snapshot_complete: bool,
}

impl JobControl {
    fn new(handle: OwnedHandle) -> Self {
        Self {
            state: Mutex::new(JobState {
                handle: Some(handle),
                termination_requested: false,
                empty_proven: false,
                retained_member_handles: Vec::new(),
                pre_termination_snapshot_complete: false,
                post_termination_snapshot_complete: false,
            }),
            #[cfg(test)]
            test_member_snapshot_failures: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            test_supplemental_snapshot_failures: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            test_wait_empty_failures: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            test_terminate_failures: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    fn inject_member_snapshot_failures(&self, count: usize) {
        self.test_member_snapshot_failures
            .store(count, AtomicOrdering::Release);
    }

    #[cfg(test)]
    fn inject_supplemental_snapshot_failures(&self, count: usize) {
        self.test_supplemental_snapshot_failures
            .store(count, AtomicOrdering::Release);
    }

    #[cfg(test)]
    fn inject_wait_empty_failures(&self, count: usize) {
        self.test_wait_empty_failures
            .store(count, AtomicOrdering::Release);
    }

    #[cfg(test)]
    fn inject_terminate_failures(&self, count: usize) {
        self.test_terminate_failures
            .store(count, AtomicOrdering::Release);
    }

    fn raw_handle(&self) -> Option<HANDLE> {
        let state = self.state.lock().ok()?;
        state.handle.as_ref().map(OwnedHandle::as_raw)
    }

    /// Retain a complete exact-member snapshot before any host termination
    /// request. Post-termination accounting alone is not sufficient because a
    /// process can leave the Job PID list just before its process handle is
    /// signalled.
    fn capture_pre_termination_members(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        if state.pre_termination_snapshot_complete {
            return Ok(());
        }
        if state.termination_requested {
            return Err(io::Error::other(
                "process Job termination was requested before an exact member snapshot completed",
            ));
        }
        if state.empty_proven {
            state.pre_termination_snapshot_complete = true;
            return Ok(());
        }
        #[cfg(test)]
        if consume_injected_failure(&self.test_member_snapshot_failures) {
            return Err(io::Error::other(
                "injected process Job member snapshot failure",
            ));
        }
        let handle = state
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "process Job is closed"))?
            .as_raw();
        let members = query_verified_job_members(handle)?;
        state.retained_member_handles.extend(members);
        state.pre_termination_snapshot_complete = true;
        Ok(())
    }

    fn capture_pre_termination_members_with_retry(&self) -> io::Result<()> {
        match self.capture_pre_termination_members() {
            Ok(()) => Ok(()),
            Err(first_error) => self.capture_pre_termination_members().map_err(|second_error| {
                io::Error::other(format!(
                    "capture exact Job members before termination failed twice ({first_error}; {second_error})"
                ))
            }),
        }
    }

    /// Refresh the retained pre-termination boundary even when an earlier
    /// snapshot already completed. This is required before retrying a failed
    /// termination request: the Job remained live during that failure window
    /// and could have acquired additional descendants.
    fn refresh_pre_termination_members(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        if state.termination_requested {
            return if state.pre_termination_snapshot_complete {
                Ok(())
            } else {
                Err(io::Error::other(
                    "process Job termination was requested before an exact member snapshot completed",
                ))
            };
        }
        if state.empty_proven {
            state.pre_termination_snapshot_complete = true;
            return Ok(());
        }
        #[cfg(test)]
        if consume_injected_failure(&self.test_member_snapshot_failures) {
            return Err(io::Error::other(
                "injected process Job member snapshot failure",
            ));
        }
        let handle = state
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "process Job is closed"))?
            .as_raw();
        let members = query_verified_job_members(handle)?;
        state.retained_member_handles.extend(members);
        state.pre_termination_snapshot_complete = true;
        Ok(())
    }

    fn refresh_pre_termination_members_with_retry(&self) -> io::Result<()> {
        match self.refresh_pre_termination_members() {
            Ok(()) => Ok(()),
            Err(first_error) => self.refresh_pre_termination_members().map_err(|second_error| {
                io::Error::other(format!(
                    "refresh exact Job members before termination retry failed twice ({first_error}; {second_error})"
                ))
            }),
        }
    }

    /// Merge a post-request snapshot into the retained evidence. This catches
    /// members created in the narrow snapshot-to-termination interval, but it
    /// deliberately does not establish pre-termination completeness by itself.
    fn retain_current_members(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        if !state.termination_requested {
            return Err(io::Error::other(
                "process Job member merge completed before termination was requested",
            ));
        }
        if state.empty_proven {
            state.post_termination_snapshot_complete = true;
            return Ok(());
        }
        #[cfg(test)]
        if consume_injected_failure(&self.test_supplemental_snapshot_failures) {
            return Err(io::Error::other(
                "injected process Job supplemental member snapshot failure",
            ));
        }
        let handle = state
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "process Job is closed"))?
            .as_raw();
        let members = query_verified_job_members(handle)?;
        state.retained_member_handles.extend(members);
        state.post_termination_snapshot_complete = true;
        Ok(())
    }

    fn require_complete_member_interval_proof(&self) -> io::Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        if !state.pre_termination_snapshot_complete {
            return Err(io::Error::other(
                "no complete exact Job member snapshot was obtained before termination",
            ));
        }
        if !state.post_termination_snapshot_complete {
            return Err(io::Error::other(
                "no complete exact Job member snapshot was obtained after termination",
            ));
        }
        Ok(())
    }

    fn wait_retained_members_until(&self, deadline: Instant) -> io::Result<()> {
        self.require_complete_member_interval_proof()?;
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        wait_process_handles_until(&state.retained_member_handles, deadline)
    }

    fn terminate(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        if state.empty_proven {
            return Ok(());
        }
        if state.termination_requested {
            return Ok(());
        }
        #[cfg(test)]
        if consume_injected_failure(&self.test_terminate_failures) {
            return Err(io::Error::other("injected process Job termination failure"));
        }
        let handle = state
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "process Job is closed"))?
            .as_raw();
        // SAFETY: handle is kept live by the locked Job state.
        if unsafe { TerminateJobObject(handle, TERMINATED_BY_HOST_EXIT_CODE) } == 0 {
            let error = io::Error::last_os_error();
            if query_active_processes(handle)? != 0 {
                return Err(error);
            }
            state.empty_proven = true;
        }
        state.termination_requested = true;
        Ok(())
    }

    fn active_processes(&self) -> io::Result<u32> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process Job state is poisoned"))?;
        if state.empty_proven {
            return Ok(0);
        }
        let handle = state
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "process Job is closed"))?
            .as_raw();
        let active = query_active_processes(handle)?;
        if active == 0 {
            state.empty_proven = true;
        }
        Ok(active)
    }

    fn close_proven_empty(&self) -> io::Result<()> {
        let handle = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| io::Error::other("process Job state is poisoned"))?;
            if !state.empty_proven {
                return Err(io::Error::other(
                    "process Job cannot close before empty membership is proven",
                ));
            }
            state.handle.take()
        };
        drop(handle);
        Ok(())
    }

    fn is_proven_empty_and_closed(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.empty_proven && state.handle.is_none()
    }

    fn wait_empty_until(&self, deadline: Instant) -> io::Result<()> {
        #[cfg(test)]
        if consume_injected_failure(&self.test_wait_empty_failures) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "injected process Job empty-proof timeout",
            ));
        }
        loop {
            if self.active_processes()? == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "process Job did not become empty before the cleanup deadline",
                ));
            }
            std::thread::sleep(
                JOB_EMPTY_POLL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn close_for_kill(&self) -> io::Result<()> {
        let handle = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| io::Error::other("process Job state is poisoned"))?;
            state.termination_requested = true;
            state.handle.take()
        };
        drop(handle);
        Ok(())
    }
}

#[cfg(test)]
fn consume_injected_failure(counter: &std::sync::atomic::AtomicUsize) -> bool {
    counter
        .fetch_update(
            AtomicOrdering::AcqRel,
            AtomicOrdering::Acquire,
            |remaining| remaining.checked_sub(1),
        )
        .is_ok()
}

fn query_verified_job_members(job: HANDLE) -> io::Result<Vec<OwnedHandle>> {
    let process_ids = query_job_process_ids(job)?;
    let mut processes = Vec::with_capacity(process_ids.len());
    for pid in process_ids {
        if let Some(process) = open_verified_job_member(job, pid)? {
            processes.push(process);
        }
    }
    Ok(processes)
}

fn open_verified_job_member(job: HANDLE, pid: u32) -> io::Result<Option<OwnedHandle>> {
    // SAFETY: OpenProcess validates the PID and returns a fresh,
    // non-inheritable process handle on success. IsProcessInJob additionally
    // requires query-limited-information access on the process handle.
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if raw.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(i32::try_from(ERROR_INVALID_PARAMETER).unwrap_or(87)) {
            // The exact member completed between the Job snapshot and
            // OpenProcess, which already proves it cannot execute.
            return Ok(None);
        }
        return Err(io::Error::new(
            error.kind(),
            format!("open Job member process {pid} for exit proof: {error}"),
        ));
    }
    // SAFETY: OpenProcess returned a fresh owned handle.
    let process = unsafe { OwnedHandle::from_raw(raw)? };
    let mut still_in_job = 0;
    // A member can exit and its numeric PID can be reused between the Job PID
    // query and OpenProcess. Verify the opened process object, not the number,
    // before accepting it as cleanup authority.
    if unsafe { IsProcessInJob(process.as_raw(), job, &mut still_in_job) } == 0 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!("verify process {pid} remains in process Job: {error}"),
        ));
    }
    Ok((still_in_job != 0).then_some(process))
}

fn query_job_process_ids(job: HANDLE) -> io::Result<Vec<u32>> {
    const INITIAL_PROCESS_SLOTS: usize = 16;
    let process_id_offset = std::mem::offset_of!(
        JOBOBJECT_BASIC_PROCESS_ID_LIST,
        ProcessIdList
    );
    let mut slots = INITIAL_PROCESS_SLOTS;
    loop {
        let byte_len = process_id_offset
            .checked_add(
                slots
                    .checked_mul(mem::size_of::<usize>())
                    .ok_or_else(|| io::Error::other("process Job member buffer overflow"))?,
            )
            .ok_or_else(|| io::Error::other("process Job member buffer overflow"))?;
        let word_len = byte_len.div_ceil(mem::size_of::<usize>());
        let mut buffer = vec![0_usize; word_len];
        let mut required = 0_u32;
        // SAFETY: `buffer` is suitably aligned for the process-id-list header,
        // and its exact byte capacity is supplied to the API.
        if unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast::<c_void>(),
                u32::try_from(byte_len)
                    .map_err(|_| io::Error::other("process Job member buffer is too large"))?,
                &mut required,
            )
        } != 0
        {
            let list =
                unsafe { &*(buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()) };
            let count = usize::try_from(list.NumberOfProcessIdsInList)
                .map_err(|_| io::Error::other("process Job member count overflow"))?;
            if count > slots {
                slots = count;
                continue;
            }
            // SAFETY: QueryInformationJobObject reported `count <= slots`
            // initialized entries in the flexible ProcessIdList tail.
            let ids = unsafe {
                std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count)
            };
            return ids
                .iter()
                .map(|pid| {
                    u32::try_from(*pid)
                        .map_err(|_| io::Error::other("process Job member PID overflow"))
                })
                .collect();
        }

        let error = io::Error::last_os_error();
        let required = usize::try_from(required).unwrap_or(usize::MAX);
        if required > byte_len {
            slots = required
                .saturating_sub(process_id_offset)
                .div_ceil(mem::size_of::<usize>())
                .max(slots.saturating_mul(2));
            continue;
        }
        return Err(error);
    }
}

fn wait_process_handles_until(
    processes: &[OwnedHandle],
    deadline: Instant,
) -> io::Result<()> {
    for process in processes {
        wait_handle_until(process.as_raw(), deadline)?;
    }
    Ok(())
}

fn query_active_processes(job: HANDLE) -> io::Result<u32> {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: job is live and accounting is valid writable storage.
    if unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast::<c_void>(),
            u32::try_from(mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>())
                .expect("JOBOBJECT_BASIC_ACCOUNTING_INFORMATION fits in u32"),
            ptr::null_mut(),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(accounting.ActiveProcesses)
    }
}

#[derive(Clone)]
enum LifecycleCompletion {
    Running,
    Reaped(ExitFact),
    Failed {
        kind: io::ErrorKind,
        message: Arc<str>,
    },
}

struct ReaderCompletion {
    receiver: mpsc::Receiver<io::Result<()>>,
}

fn start_reader(
    runtime: &tokio::runtime::Handle,
    handle: OwnedHandle,
    stream: OutputStream,
    output: Arc<OutputBuffer>,
) -> ReaderCompletion {
    let (sender, receiver) = mpsc::sync_channel(1);
    runtime.spawn_blocking(move || {
        let _ = sender.send(read_stream(handle, stream, output));
    });
    ReaderCompletion { receiver }
}

fn read_stream(
    handle: OwnedHandle,
    stream: OutputStream,
    output: Arc<OutputBuffer>,
) -> io::Result<()> {
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    loop {
        let mut read = 0_u32;
        // SAFETY: buffer is writable for its full length and handle is a live
        // synchronous pipe read handle owned by this worker.
        let result = unsafe {
            ReadFile(
                handle.as_raw(),
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).expect("reader buffer fits in u32"),
                &mut read,
                ptr::null_mut(),
            )
        };
        if result == 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error().map(|code| code as u32),
                Some(ERROR_BROKEN_PIPE | ERROR_NO_DATA)
            ) {
                return Ok(());
            }
            return Err(error);
        }
        if read == 0 {
            return Ok(());
        }
        output.push(stream, &buffer[..read as usize]);
    }
}

fn run_lifecycle(
    process: Arc<OwnedHandle>,
    job: Arc<JobControl>,
    readers: Vec<ReaderCompletion>,
    pseudoconsole: Option<Arc<PseudoConsoleControl>>,
    deadline: Instant,
) -> io::Result<ExitFact> {
    wait_handle_until(process.as_raw(), deadline)?;
    let mut exit_code = 0_u32;
    // SAFETY: process is signaled and its exact handle remains live.
    if unsafe { GetExitCodeProcess(process.as_raw(), &mut exit_code) } == 0 {
        return Err(io::Error::last_os_error());
    }

    if job.active_processes()? != 0 {
        job.terminate()?;
        job.wait_empty_until(deadline)?;
    }
    job.close_proven_empty()?;
    let mut cleanup_errors = Vec::new();
    if let Some(pseudoconsole) = pseudoconsole {
        let close_deadline = deadline.min(
            Instant::now()
                .checked_add(CONPTY_NATURAL_CLOSE_WAIT)
                .unwrap_or(deadline),
        );
        if let Err(error) = pseudoconsole.close_until(close_deadline) {
            cleanup_errors.push(format!("close pseudoconsole: {error}"));
        }
        drain_pty_readers_after_close(readers, deadline);
    } else {
        finish_readers(readers, deadline)?;
    }

    Ok(ExitFact {
        code: Some(exit_code as i32),
        signal: None,
        cleanup_errors,
    })
}

fn lifecycle_failure_message(
    error: io::Error,
    process: &OwnedHandle,
    job: &JobControl,
) -> String {
    let deadline = Instant::now()
        .checked_add(CLEANUP_TIMEOUT)
        .unwrap_or_else(Instant::now);
    let mut cleanup_errors = Vec::new();
    if let Err(cleanup_error) = job.terminate() {
        cleanup_errors.push(format!("terminate process Job: {cleanup_error}"));
    }
    if let Err(cleanup_error) = wait_handle_until(process.as_raw(), deadline) {
        cleanup_errors.push(format!("wait exact process handle: {cleanup_error}"));
    }
    if let Err(cleanup_error) = job.wait_empty_until(deadline) {
        cleanup_errors.push(format!("prove process Job empty: {cleanup_error}"));
    } else if let Err(cleanup_error) = job.close_proven_empty() {
        cleanup_errors.push(format!("close proven-empty process Job: {cleanup_error}"));
    }
    if !cleanup_errors.is_empty() {
        let _ = job.close_for_kill();
        format!(
            "{error}; fail-closed lifecycle cleanup: {}",
            cleanup_errors.join("; ")
        )
    } else {
        error.to_string()
    }
}

fn finish_readers(readers: Vec<ReaderCompletion>, caller_deadline: Instant) -> io::Result<()> {
    let deadline = caller_deadline.min(
        Instant::now()
            .checked_add(POST_EXIT_READER_DRAIN)
            .unwrap_or(caller_deadline),
    );
    let mut first_error = None;
    for reader in readers {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let result = if remaining.is_zero() {
            Err(mpsc::RecvTimeoutError::Timeout)
        } else {
            reader.receiver.recv_timeout(remaining)
        };
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
            Ok(Err(_)) => {}
            Err(mpsc::RecvTimeoutError::Timeout) if first_error.is_none() => {
                first_error = Some(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "output reader timed out",
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) if first_error.is_none() => {
                first_error = Some(io::Error::other(
                    "output reader ended without publishing a result",
                ));
            }
            Err(_) => {}
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn drain_pty_readers_after_close(
    readers: Vec<ReaderCompletion>,
    caller_deadline: Instant,
) {
    let deadline = caller_deadline.min(
        Instant::now()
            .checked_add(POST_EXIT_READER_DRAIN)
            .unwrap_or(caller_deadline),
    );
    for reader in readers {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        match reader.receiver.recv_timeout(remaining) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return,
        }
    }
}

struct WindowsOwner {
    pid: u32,
    process: Arc<OwnedHandle>,
    job: Arc<JobControl>,
    stdin: Arc<tokio::sync::Mutex<Option<OwnedHandle>>>,
    pseudoconsole: Option<Arc<PseudoConsoleControl>>,
    pty_input_close_not_before: Option<Instant>,
    completion: watch::Receiver<LifecycleCompletion>,
}

impl Drop for WindowsOwner {
    fn drop(&mut self) {
        let _ = self.job.close_for_kill();
        if let Some(pseudoconsole) = &self.pseudoconsole {
            let _ = pseudoconsole.begin_close();
        }
    }
}

#[async_trait]
impl PlatformProcess for WindowsOwner {
    fn pid(&self) -> u32 {
        self.pid
    }

    async fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let stdin = Arc::clone(&self.stdin).lock_owned().await;
        if stdin.is_none() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdin is closed"));
        }
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            let handle = stdin
                .as_ref()
                .expect("stdin was validated before entering the blocking writer")
                .as_raw();
            write_all(handle, &bytes)
        })
            .await
            .map_err(|error| io::Error::other(format!("stdin writer task failed: {error}")))?
    }

    async fn close_stdin(&self) -> io::Result<()> {
        let mut stdin = self.stdin.lock().await;
        if stdin.is_none() {
            return Ok(());
        }
        if let Some(not_before) = self.pty_input_close_not_before {
            let remaining = not_before.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
            let stdin = stdin
                .take()
                .expect("ConPTY stdin was validated before the EOF write");
            tokio::task::spawn_blocking(move || {
                // In the default ConPTY cooked-input contract, CR submits any
                // pending line and SUB is the Windows console/CRT EOF
                // character. Closing the input pipe alone is observed by
                // console clients as CTRL_CLOSE/CONTROL_C_EXIT rather than a
                // successful stdin EOF. Arbitrary raw-mode clients cannot
                // expose a truthful generic EOF operation through ConPTY.
                write_all(stdin.as_raw(), b"\r\n\x1a")?;
                std::thread::sleep(Duration::from_millis(25));
                Ok::<(), io::Error>(())
            })
            .await
            .map_err(|error| io::Error::other(format!("PTY EOF writer task failed: {error}")))??;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "ConPTY cannot prove generic stdin EOF; a default cooked-console EOF \
                 sequence was delivered before closing input",
            ));
        }
        stdin.take();
        Ok(())
    }

    async fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.pseudoconsole
            .as_ref()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "pipe transport does not support terminal resize",
                )
            })?
            .resize(cols, rows)
    }

    async fn interrupt(&self) -> io::Result<()> {
        if self.pseudoconsole.is_some() {
            // In ConPTY's VT input contract, ETX is the terminal Ctrl+C
            // keystroke. Tree-wide escalation remains owned by the process
            // Job if the foreground application ignores it.
            self.write(b"\x03").await
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "CREATE_NO_WINDOW pipe processes have no truthful console interrupt contract",
            ))
        }
    }

    async fn terminate(&self) -> io::Result<()> {
        self.job.terminate()
    }

    async fn force_kill(&self) -> io::Result<()> {
        self.job.terminate()
    }

    async fn wait_reaped(&self, deadline: Instant) -> io::Result<ExitFact> {
        let _keep_exact_process_handle_alive = &self.process;
        let mut completion = self.completion.clone();
        loop {
            match completion.borrow().clone() {
                LifecycleCompletion::Running => {}
                LifecycleCompletion::Reaped(fact) => return Ok(fact),
                LifecycleCompletion::Failed { kind, message } => {
                    return Err(io::Error::new(kind, message.to_string()));
                }
            }
            match tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                completion.changed(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    let _ = self.job.close_for_kill();
                    return Err(io::Error::other(
                        "Windows lifecycle worker ended without a result",
                    ));
                }
                Err(_) => {
                    let _ = self.job.close_for_kill();
                    if let Some(pseudoconsole) = &self.pseudoconsole {
                        let _ = pseudoconsole.begin_close();
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "exact Windows process wait timed out",
                    ));
                }
            }
        }
    }
}

fn write_all(handle: HANDLE, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let chunk_len = bytes.len().min(WRITE_CHUNK_BYTES);
        let mut written = 0_u32;
        // SAFETY: bytes points to chunk_len readable bytes and handle is kept
        // live by the Arc owned by the blocking writer.
        if unsafe {
            WriteFile(
                handle,
                bytes.as_ptr(),
                u32::try_from(chunk_len).expect("chunk length is bounded to u32"),
                &mut written,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "Windows stdin pipe accepted zero bytes",
            ));
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

fn wait_handle_until(handle: HANDLE, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let milliseconds = if remaining.is_zero() {
            0
        } else {
            remaining
                .as_millis()
                .saturating_add(1)
                .min((u32::MAX - 1) as u128) as u32
        };
        // SAFETY: handle is a live process handle for the duration of the wait.
        match unsafe { WaitForSingleObject(handle, milliseconds) } {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT if Instant::now() < deadline => {}
            WAIT_TIMEOUT => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "exact process termination timed out",
                ));
            }
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            result => {
                return Err(io::Error::other(format!(
                    "unexpected exact process wait result: {result}"
                )));
            }
        }
    }
}

fn handle_is_signaled(handle: HANDLE) -> io::Result<bool> {
    // SAFETY: handle is live for this nonblocking probe.
    match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        result => Err(io::Error::other(format!(
            "unexpected process liveness probe result: {result}"
        ))),
    }
}

fn ensure_setup_active(deadline: Instant, cancellation: &StartCancellation) -> io::Result<()> {
    if cancellation.cancelled.load(AtomicOrdering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Windows start future was cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Windows spawn transaction exceeded its setup deadline",
        ));
    }
    Ok(())
}

struct PreparedCommand {
    application: Vec<u16>,
    command_line: Vec<u16>,
    cwd: Vec<u16>,
    environment: Vec<u16>,
}

impl PreparedCommand {
    fn new(request: &NormalizedProcessRequest) -> Result<Self, ProcessError> {
        let (program, args) = command_argv(&request.command)?;
        let application = encode_nul_terminated(&program, "program")?;
        let command_line = encode_command_line(&program, &args)?;
        let cwd = encode_nul_terminated(request.cwd.as_os_str(), "working directory").map_err(
            |error| ProcessError::InvalidWorkingDirectory {
                path: request.cwd.clone(),
                reason: error.to_string(),
            },
        )?;
        let environment = encode_environment(&request.env)?;
        Ok(Self {
            application,
            command_line,
            cwd,
            environment,
        })
    }
}

fn command_argv(spec: &CommandSpec) -> Result<(OsString, Vec<OsString>), ProcessError> {
    match spec {
        CommandSpec::Program { program, args } => Ok((program.clone(), args.clone())),
        CommandSpec::Shell {
            shell: ShellKind::PowerShell,
            script,
        } => Ok((
            powershell_executable()?,
            vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-Command"),
                OsString::from(powershell_payload(script)),
            ],
        )),
        CommandSpec::Shell {
            shell: ShellKind::PowerShellLiteral,
            script,
        } => Ok((
            powershell_executable()?,
            vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-Command"),
                OsString::from(script),
            ],
        )),
        CommandSpec::Shell {
            shell: ShellKind::Posix,
            ..
        } => Err(invalid_command(
            "POSIX shell text cannot be reinterpreted as PowerShell on Windows",
        )),
    }
}

fn powershell_executable() -> Result<OsString, ProcessError> {
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: `buffer` is writable for its declared length and the API writes a
    // NUL-terminated Windows directory path or returns zero on failure.
    let length = unsafe {
        GetWindowsDirectoryW(
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).expect("Windows directory buffer fits in u32"),
        )
    };
    if length == 0 {
        return Err(spawn_failed(io::Error::last_os_error()));
    }
    let length = usize::try_from(length)
        .map_err(|_| invalid_command("Windows directory length did not fit usize"))?;
    if length >= buffer.len() {
        return Err(invalid_command(
            "Windows directory exceeded the fixed command-resolution buffer",
        ));
    }
    buffer.truncate(length);
    let executable = std::path::PathBuf::from(OsString::from_wide(&buffer))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if !executable.is_file() {
        return Err(ProcessError::SpawnFailed {
            failure: SpawnFailure {
                code: "powershell_unavailable".to_owned(),
                message: format!(
                    "trusted Windows PowerShell executable is unavailable: {}",
                    executable.display()
                ),
            },
        });
    }
    Ok(executable.into_os_string())
}

fn powershell_payload(script: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         [Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)\n\
         [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n\
         $OutputEncoding = [Console]::OutputEncoding\n\
         $PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'\n\
         $global:LASTEXITCODE = $null\n\
         try {{\n\
         & {{\n\
         {script}\n\
         $nomifunSucceeded = $?\n\
         $nomifunLastExitCode = $global:LASTEXITCODE\n\
         if ($null -ne $nomifunLastExitCode -and -not $nomifunSucceeded) {{ exit $nomifunLastExitCode }}\n\
         if (-not $nomifunSucceeded) {{ exit 1 }}\n\
         }}\n\
         $commandSucceeded = $?\n\
         if (-not $commandSucceeded) {{\n\
           if ($null -ne $global:LASTEXITCODE) {{ exit $global:LASTEXITCODE }}\n\
           exit 1\n\
         }}\n\
         exit 0\n\
         }} catch {{\n\
         [Console]::Error.WriteLine($_.Exception.Message)\n\
         exit 1\n\
         }}"
    )
}

fn encode_command_line(program: &OsStr, args: &[OsString]) -> Result<Vec<u16>, ProcessError> {
    let mut command_line = Vec::new();
    append_quoted(program, &mut command_line)?;
    for arg in args {
        command_line.push(b' ' as u16);
        append_quoted(arg, &mut command_line)?;
    }
    command_line.push(0);
    if command_line.len() > MAX_COMMAND_LINE_UNITS {
        return Err(invalid_command(
            "Windows command line exceeds 32,767 UTF-16 code units",
        ));
    }
    Ok(command_line)
}

fn append_quoted(arg: &OsStr, command_line: &mut Vec<u16>) -> Result<(), ProcessError> {
    let encoded = arg.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(invalid_command(
            "Windows command-line arguments cannot contain NUL",
        ));
    }
    let requires_quotes = encoded.is_empty()
        || encoded.iter().any(|unit| {
            matches!(
                *unit,
                value if value == b' ' as u16
                    || value == b'\t' as u16
                    || value == b'\n' as u16
                    || value == 0x0B
                    || value == b'"' as u16
            )
        });
    if !requires_quotes {
        command_line.extend_from_slice(&encoded);
        return Ok(());
    }

    command_line.push(b'"' as u16);
    let mut index = 0;
    while index < encoded.len() {
        let mut backslashes = 0_usize;
        while index < encoded.len() && encoded[index] == b'\\' as u16 {
            backslashes += 1;
            index += 1;
        }
        if index == encoded.len() {
            command_line.extend(std::iter::repeat_n(
                b'\\' as u16,
                backslashes.saturating_mul(2),
            ));
            break;
        }
        if encoded[index] == b'"' as u16 {
            command_line.extend(std::iter::repeat_n(
                b'\\' as u16,
                backslashes.saturating_mul(2).saturating_add(1),
            ));
        } else {
            command_line.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
        }
        command_line.push(encoded[index]);
        index += 1;
    }
    command_line.push(b'"' as u16);
    Ok(())
}

struct EnvironmentEntry {
    key: OsString,
    value: OsString,
}

fn encode_environment(
    overrides: &std::collections::BTreeMap<OsString, OsString>,
) -> Result<Vec<u16>, ProcessError> {
    encode_environment_from(std::env::vars_os(), overrides)
}

fn encode_environment_from(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
    overrides: &std::collections::BTreeMap<OsString, OsString>,
) -> Result<Vec<u16>, ProcessError> {
    let mut entries = Vec::<EnvironmentEntry>::new();
    for (key, value) in inherited {
        if dangerous_inherited_environment(&key) {
            continue;
        }
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| compare_os_case_insensitive(&entry.key, &key) == Ordering::Equal)
        {
            entry.key = key;
            entry.value = value;
        } else {
            entries.push(EnvironmentEntry { key, value });
        }
    }

    for (key, value) in overrides {
        validate_environment_override(key, value)?;
        if dangerous_inherited_environment(key) {
            return Err(invalid_command(format!(
                "environment override {key:?} is forbidden at process boundary"
            )));
        }
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| compare_os_case_insensitive(&entry.key, key) == Ordering::Equal)
        {
            entry.key = key.clone();
            entry.value = value.clone();
        } else {
            entries.push(EnvironmentEntry {
                key: key.clone(),
                value: value.clone(),
            });
        }
    }

    entries.sort_by(|left, right| {
        compare_os_case_insensitive(&left.key, &right.key)
            .then_with(|| compare_os_raw(&left.key, &right.key))
    });
    let mut block = Vec::new();
    for entry in entries {
        append_environment_component(&entry.key, "environment key", &mut block)?;
        block.push(b'=' as u16);
        append_environment_component(&entry.value, "environment value", &mut block)?;
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}

fn dangerous_inherited_environment(key: &OsStr) -> bool {
    [
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "DYLD_FRAMEWORK_PATH",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "NODE_OPTIONS",
        "NODE_INSPECT",
        "NODE_DEBUG",
        "CLAUDECODE",
    ]
    .iter()
    .any(|candidate| {
        compare_os_case_insensitive(key, OsStr::new(candidate)) == Ordering::Equal
    })
}

fn validate_environment_override(key: &OsStr, value: &OsStr) -> Result<(), ProcessError> {
    let encoded_key = key.encode_wide().collect::<Vec<_>>();
    if encoded_key.is_empty() {
        return Err(invalid_command("environment keys cannot be empty"));
    }
    if encoded_key
        .iter()
        .any(|unit| *unit == 0 || *unit == b'=' as u16)
    {
        return Err(invalid_command(
            "environment keys cannot contain NUL or '='",
        ));
    }
    if value.encode_wide().any(|unit| unit == 0) {
        return Err(invalid_command(
            "environment values cannot contain NUL",
        ));
    }
    Ok(())
}

fn append_environment_component(
    value: &OsStr,
    label: &'static str,
    destination: &mut Vec<u16>,
) -> Result<(), ProcessError> {
    for unit in value.encode_wide() {
        if unit == 0 {
            return Err(invalid_command(format!("{label} cannot contain NUL")));
        }
        destination.push(unit);
    }
    Ok(())
}

fn compare_os_case_insensitive(left: &OsStr, right: &OsStr) -> Ordering {
    let left = left.encode_wide().collect::<Vec<_>>();
    let right = right.encode_wide().collect::<Vec<_>>();
    let Some(left_len) = i32::try_from(left.len()).ok() else {
        return left.cmp(&right);
    };
    let Some(right_len) = i32::try_from(right.len()).ok() else {
        return left.cmp(&right);
    };
    // SAFETY: the explicit lengths bound both readable UTF-16 buffers.
    match unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left_len,
            right.as_ptr(),
            right_len,
            1,
        )
    } {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_GREATER_THAN => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

fn compare_os_raw(left: &OsStr, right: &OsStr) -> Ordering {
    left.encode_wide()
        .collect::<Vec<_>>()
        .cmp(&right.encode_wide().collect::<Vec<_>>())
}

fn encode_nul_terminated(value: &OsStr, label: &'static str) -> Result<Vec<u16>, ProcessError> {
    let mut encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(invalid_command(format!("{label} cannot contain NUL")));
    }
    encoded.push(0);
    Ok(encoded)
}

fn enforce_sandbox(request: &NormalizedProcessRequest) -> Result<(), ProcessError> {
    match &request.capability.sandbox {
        SandboxPolicy::UnrestrictedLocalOwner => Ok(()),
        SandboxPolicy::DenySpawn => Err(ProcessError::CapabilityDenied {
            path: request.cwd.clone(),
            reason: "process is denied by the sandbox policy".to_owned(),
        }),
        SandboxPolicy::MacSeatbelt { .. } => Err(ProcessError::CapabilityDenied {
            path: request.cwd.clone(),
            reason: "macOS Seatbelt cannot authorize Windows process".to_owned(),
        }),
    }
}

fn invalid_command(reason: impl Into<String>) -> ProcessError {
    ProcessError::InvalidCommand {
        reason: reason.into(),
    }
}

fn spawn_failed(error: io::Error) -> ProcessError {
    ProcessError::SpawnFailed {
        failure: SpawnFailure {
            code: "spawn_failed".to_owned(),
            message: error.to_string(),
        },
    }
}

fn start_lost(
    code: &'static str,
    message: String,
    last_known: Option<ProcessSnapshot>,
    cleanup: CleanupReport,
) -> ProcessError {
    ProcessError::StartLost {
        failure: SpawnFailure {
            code: code.to_owned(),
            message,
        },
        last_known,
        cleanup,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        ffi::{OsStr, OsString},
        os::windows::ffi::OsStringExt,
        sync::Mutex,
        time::Duration,
    };

    use serial_test::serial;
    use tempfile::TempDir;
    use windows_sys::Win32::Foundation::HANDLE;

    use super::*;
    use crate::{CapabilityPolicy, ProcessOwner, ProcessPolicy, Transport};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SpawnAuditEvent {
        Created,
        Assigned,
        Resumed,
    }

    #[test]
    fn process_job_rejects_zero_pid_without_opening_a_process() {
        let error = match WindowsProcessJob::attach(0) {
            Ok(_) => panic!("PID 0 must be rejected before OpenProcess is attempted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn job_member_verification_rejects_an_opened_process_outside_the_job() {
        let job = JobControl::new(create_process_job().expect("empty process Job should be created"));
        let job_handle = job
            .raw_handle()
            .expect("new process Job should retain its handle");

        let member = open_verified_job_member(job_handle, std::process::id())
            .expect("current process membership should be queryable");

        assert!(
            member.is_none(),
            "an opened process object outside the Job must not be accepted after a PID race"
        );
    }

    #[test]
    fn termination_retry_refreshes_an_already_complete_pre_snapshot() {
        let job = JobControl::new(create_process_job().expect("empty process Job should be created"));
        job.capture_pre_termination_members()
            .expect("initial exact-member snapshot should complete");
        job.inject_member_snapshot_failures(1);

        let error = job
            .refresh_pre_termination_members()
            .expect_err("retry refresh must query again instead of accepting the old snapshot");

        assert!(
            error
                .to_string()
                .contains("injected process Job member snapshot failure"),
            "refresh should exercise a new member query: {error}"
        );
        job.close_for_kill()
            .expect("empty test Job should close through kill-on-close");
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial]
    async fn every_post_create_child_spawn_failure_is_reaped_by_the_bounded_poller() {
        let temporary = TempDir::new().expect("temporary marker directory should be created");
        let cases = [
            (TestChildSpawnFault::MissingId, "missing child id"),
            (
                TestChildSpawnFault::MissingRawHandle,
                "missing child process handle",
            ),
            (TestChildSpawnFault::MissingJobHandle, "missing Job handle"),
            (
                TestChildSpawnFault::Assignment,
                "AssignProcessToJobObject failure",
            ),
            (
                TestChildSpawnFault::DuplicateHandle,
                "handle duplication failure",
            ),
        ];

        for (index, (fault, expected_message)) in cases.into_iter().enumerate() {
            let marker = temporary.path().join(format!("must-not-run-{index}.marker"));
            let mut command = tokio::process::Command::new(command_shell());
            command
                .args([
                    OsString::from("/D"),
                    OsString::from("/S"),
                    OsString::from("/C"),
                    OsString::from(format!(
                        ">\"{}\" echo resumed & ping -n 30 127.0.0.1 >NUL",
                        marker.display()
                    )),
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let admitted_before =
                crate::platform::poller::platform_lifecycle_metrics().admitted;
            let completion = inject_test_child_spawn_fault(fault);

            let error = match spawn_child_process(command, false) {
                Ok(_) => panic!("injected post-CreateProcess failure must reject the spawn"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains(expected_message),
                "unexpected injected failure for {fault:?}: {error}"
            );
            assert!(
                error
                    .to_string()
                    .contains("exact cleanup retained by the bounded platform poller"),
                "post-create failure did not report durable cleanup retention: {error}"
            );
            assert_eq!(
                completion
                    .recv_timeout(Duration::from_secs(5))
                    .expect("failed-spawn cleanup must publish a bounded result"),
                true,
                "{fault:?} did not reach exact direct-child/Job cleanup proof"
            );
            let release_deadline = Instant::now() + Duration::from_secs(1);
            while crate::platform::poller::platform_lifecycle_metrics().admitted
                > admitted_before
                && Instant::now() < release_deadline
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(
                crate::platform::poller::platform_lifecycle_metrics().admitted
                    <= admitted_before,
                "{fault:?} retained its lifecycle slot after exact cleanup"
            );
            assert!(
                !marker.exists(),
                "{fault:?} resumed the suspended child and executed user code"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn child_job_registry_removes_the_exact_entry_at_terminal_cleanup() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "exit 0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with registry authority should start");
        let pid = child.id().expect("managed child should expose its PID");
        child.wait().await.expect("direct child should reap");
        cleanup
            .wait()
            .await
            .expect("process Job should publish exact cleanup");

        assert!(
            child_process_job(pid)
                .expect("registry lookup should remain healthy")
                .is_none(),
            "terminal child-process Job entry was not retired"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn child_cleanup_survives_a_temporary_member_snapshot_failure() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_member_snapshot_failures(1);

        cleanup
            .shutdown(&mut child)
            .await
            .expect("termination must converge despite one member snapshot failure");

        assert!(
            child
                .try_wait()
                .expect("reaped child status should be readable")
                .is_some(),
            "snapshot failure skipped process Job termination or direct-child reap"
        );
        assert_eq!(
            process
                .job
                .test_member_snapshot_failures
                .load(AtomicOrdering::Acquire),
            0,
            "the injected member snapshot failure was not exercised"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn child_cleanup_recovers_a_temporary_supplemental_snapshot_failure() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_supplemental_snapshot_failures(1);

        cleanup
            .shutdown(&mut child)
            .await
            .expect("a later complete post-termination snapshot should recover one query failure");

        assert_eq!(
            process
                .job
                .test_supplemental_snapshot_failures
                .load(AtomicOrdering::Acquire),
            0,
            "the injected supplemental snapshot failure was not exercised"
        );
        assert!(
            matches!(process.completion.borrow().as_ref(), Some(Ok(()))),
            "a complete retry must publish successful interval proof"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn persistent_supplemental_snapshot_failure_kills_but_fails_closed() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_supplemental_snapshot_failures(3);

        let error = cleanup
            .shutdown(&mut child)
            .await
            .expect_err("persistent interval-query failure must never publish cleanup success");

        assert!(
            child
                .try_wait()
                .expect("reaped child status should be readable")
                .is_some(),
            "proof failure must not suppress TerminateJobObject or kill-on-close"
        );
        assert!(
            error
                .to_string()
                .contains("no complete exact Job member snapshot was obtained after termination"),
            "persistent supplemental failure should report missing interval proof: {error}"
        );
        assert!(
            matches!(process.completion.borrow().as_ref(), Some(Err(_))),
            "persistent missing interval proof must be published as failure"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn reaper_fails_closed_without_any_complete_member_snapshot() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_member_snapshot_failures(2);

        let error = cleanup
            .shutdown(&mut child)
            .await
            .expect_err("cleanup must remain unproven when both snapshots fail");

        assert!(
            error
                .to_string()
                .contains("no complete exact Job member snapshot was obtained"),
            "persistent snapshot failure should report the missing proof: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn child_cleanup_retries_a_temporary_terminate_failure() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_terminate_failures(1);

        cleanup
            .shutdown(&mut child)
            .await
            .expect("termination retry should converge after one temporary failure");

        assert_eq!(
            process
                .job
                .test_terminate_failures
                .load(AtomicOrdering::Acquire),
            0,
            "the injected TerminateJobObject failure was not exercised"
        );
        assert!(
            child
                .try_wait()
                .expect("reaped child status should be readable")
                .is_some(),
            "termination retry did not terminate the direct child"
        );
        assert!(
            matches!(process.completion.borrow().as_ref(), Some(Ok(()))),
            "pre/post exact handles should make the termination retry provable"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn persistent_terminate_failure_kills_on_close_but_remains_unproven() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_terminate_failures(2);

        let error = cleanup
            .shutdown(&mut child)
            .await
            .expect_err("close-only fallback cannot claim a complete exact-member proof");

        assert!(
            child
                .try_wait()
                .expect("reaped child status should be readable")
                .is_some(),
            "persistent termination failure left the direct child alive"
        );
        assert!(
            error
                .to_string()
                .contains("no complete exact Job member snapshot was obtained after termination"),
            "persistent termination failure should fail closed on missing interval proof: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn reaper_fallback_publishes_success_after_exact_handles_terminate() {
        let mut command = tokio::process::Command::new(command_shell());
        command
            .args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let (mut child, cleanup) = spawn_child_process(command, false)
            .expect("managed child with process Job should start");
        let process = cleanup
            .process
            .as_ref()
            .expect("managed child should retain Job cleanup authority");
        process.job.inject_wait_empty_failures(1);

        cleanup
            .shutdown(&mut child)
            .await
            .expect("kill-on-close fallback should settle verified exact handles");

        assert_eq!(
            process
                .job
                .test_wait_empty_failures
                .load(AtomicOrdering::Acquire),
            0,
            "the injected normal Job proof failure was not exercised"
        );
        assert!(
            matches!(process.completion.borrow().as_ref(), Some(Ok(()))),
            "a recovered fallback must publish success instead of a sticky error"
        );
    }

    #[test]
    fn finish_readers_tolerates_post_exit_scheduler_jitter() {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            sender
                .send(Ok(()))
                .expect("reader completion receiver should remain live");
        });

        let result = finish_readers(
            vec![ReaderCompletion { receiver }],
            Instant::now() + Duration::from_secs(2),
        );
        worker.join().expect("reader completion worker should join");

        result.expect("brief scheduler jitter after exit must not make cleanup unproven");
    }

    struct AuditFacade {
        events: Mutex<Vec<SpawnAuditEvent>>,
        fail_assignment: bool,
        created: Mutex<Option<CreatedProcess>>,
    }

    struct CreatedProcess {
        pid: u32,
        process: OwnedHandle,
    }

    impl AuditFacade {
        fn assignment_failure() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                fail_assignment: true,
                created: Mutex::new(None),
            }
        }

        fn successful() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                fail_assignment: false,
                created: Mutex::new(None),
            }
        }

        fn events(&self) -> Vec<SpawnAuditEvent> {
            self.events
                .lock()
                .expect("spawn audit event mutex should not be poisoned")
                .clone()
        }

        fn created_pid(&self) -> Option<u32> {
            self.created
                .lock()
                .expect("created process mutex should not be poisoned")
                .as_ref()
                .map(|created| created.pid)
        }

    fn created_process_is_signaled(&self) -> io::Result<bool> {
            let created = self
                .created
                .lock()
                .map_err(|_| io::Error::other("created process mutex is poisoned"))?;
            let created = created
                .as_ref()
                .ok_or_else(|| io::Error::other("CreateProcessW was not observed"))?;
            handle_is_signaled(created.process.as_raw())
        }

        fn wait_created_process_terminated(&self) -> io::Result<()> {
            let created = self
                .created
                .lock()
                .map_err(|_| io::Error::other("created process mutex is poisoned"))?;
            let created = created
                .as_ref()
                .ok_or_else(|| io::Error::other("CreateProcessW was not observed"))?;
            wait_handle_until(
                created.process.as_raw(),
                Instant::now() + Duration::from_secs(5),
            )
        }
    }

    impl Win32Facade for AuditFacade {
        fn process_created(
            &self,
            pid: u32,
            process: HANDLE,
            _thread: HANDLE,
        ) -> io::Result<()> {
            let process = duplicate_non_inheritable(process)?;
            self.events
                .lock()
                .map_err(|_| io::Error::other("spawn audit event mutex is poisoned"))?
                .push(SpawnAuditEvent::Created);
            *self
                .created
                .lock()
                .map_err(|_| io::Error::other("created process mutex is poisoned"))? =
                Some(CreatedProcess { pid, process });
            Ok(())
        }

        fn assign_process_to_job(&self, job: HANDLE, process: HANDLE) -> io::Result<()> {
            self.events
                .lock()
                .map_err(|_| io::Error::other("spawn audit event mutex is poisoned"))?
                .push(SpawnAuditEvent::Assigned);
            if self.fail_assignment {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected AssignProcessToJobObject failure",
                ))
            } else {
                SystemWin32.assign_process_to_job(job, process)
            }
        }

        fn resume_thread(&self, thread: HANDLE) -> io::Result<u32> {
            self.events
                .lock()
                .map_err(|_| io::Error::other("spawn audit event mutex is poisoned"))?
                .push(SpawnAuditEvent::Resumed);
            SystemWin32.resume_thread(thread)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn assignment_failure_never_resumes_or_executes_user_code() {
        let temporary = TempDir::new().expect("temporary marker directory should be created");
        let marker = temporary.path().join("must-not-exist.marker");
        let facade = Arc::new(AuditFacade::assignment_failure());
        let request = program_request(
            command_shell(),
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from(format!(">\"{}\" echo resumed", marker.display())),
            ],
        );
        let output = Arc::new(OutputBuffer::new(4096));

        let result = spawn_pipe_inner(request, output, facade.clone()).await;

        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("injected Job assignment failure must reject start"),
        };
        let ProcessError::SpawnFailed { failure } = error else {
            panic!("pre-resume assignment failure must be stable SpawnFailed, got {error:?}");
        };
        assert_eq!(failure.code, "spawn_failed");
        assert!(
            failure
                .message
                .contains("injected AssignProcessToJobObject failure"),
            "unexpected spawn failure message: {}",
            failure.message
        );
        assert_eq!(
            facade.events(),
            vec![SpawnAuditEvent::Created, SpawnAuditEvent::Assigned],
            "ResumeThread must never be attempted after assignment failure"
        );
        assert!(
            !marker.exists(),
            "the suspended child executed user code despite assignment failure"
        );
        assert!(
            facade
                .created_process_is_signaled()
                .expect("exact process handle liveness probe should succeed"),
            "the suspended child was not reaped before SpawnFailed returned"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn conpty_assignment_failure_never_resumes_or_executes_user_code() {
        let temporary = TempDir::new().expect("temporary marker directory should be created");
        let marker = temporary.path().join("conpty-must-not-exist.marker");
        let facade = Arc::new(AuditFacade::assignment_failure());
        let request = program_request(
            command_shell(),
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from(format!(">\"{}\" echo resumed", marker.display())),
            ],
        );

        let result = spawn_pty_inner(
            request,
            Arc::new(OutputBuffer::new(4096)),
            facade.clone(),
            80,
            24,
        )
        .await;

        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("injected ConPTY Job assignment failure must reject start"),
        };
        assert!(matches!(error, ProcessError::SpawnFailed { .. }));
        assert_eq!(
            facade.events(),
            vec![SpawnAuditEvent::Created, SpawnAuditEvent::Assigned]
        );
        assert!(!marker.exists(), "suspended ConPTY child executed user code");
        assert!(
            facade
                .created_process_is_signaled()
                .expect("exact ConPTY process liveness probe should succeed")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn spawn_order_is_create_then_assign_then_resume() {
        let facade = Arc::new(AuditFacade::successful());
        let request = program_request(
            command_shell(),
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from("exit 0"),
            ],
        );
        let output = Arc::new(OutputBuffer::new(4096));

        let spawned = spawn_pipe_inner(request, output, facade.clone())
            .await
            .expect("audited helper should start");
        let fact = spawned
            .owner
            .wait_reaped(Instant::now() + Duration::from_secs(5))
            .await
            .expect("audited helper should be reaped");

        assert_eq!(fact.code, Some(0));
        assert_eq!(fact.signal, None);
        assert_eq!(
            facade.events(),
            vec![
                SpawnAuditEvent::Created,
                SpawnAuditEvent::Assigned,
                SpawnAuditEvent::Resumed,
            ]
        );
        assert!(facade.created_pid().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn dropping_start_future_after_create_cannot_resume_later() {
        let temporary = TempDir::new().expect("temporary marker directory should be created");
        let marker = temporary.path().join("drop-must-not-resume.marker");
        let facade = Arc::new(PausedAssignFacade::new());
        let request = program_request(
            command_shell(),
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from(format!(">\"{}\" echo resumed", marker.display())),
            ],
        );
        let output = Arc::new(OutputBuffer::new(4096));
        let start = tokio::spawn(spawn_pipe_inner(request, output, facade.clone()));

        tokio::time::timeout(Duration::from_secs(5), facade.wait_until_assign_entered())
            .await
            .expect("spawn transaction should reach the injected assignment pause");
        start.abort();
        let _ = start.await;
        facade.release_assignment();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if facade.events().contains(&SpawnAuditEvent::Assigned) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("cancelled blocking transaction should finish assignment cleanup");

        assert_eq!(
            facade.events(),
            vec![SpawnAuditEvent::Created, SpawnAuditEvent::Assigned],
            "a dropped start future permitted a late ResumeThread"
        );
        assert!(!marker.exists(), "dropped start executed user code");
        facade
            .wait_created_process_terminated()
            .expect("cancelled start must leave no exact process behind");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn dropping_conpty_start_after_create_cannot_resume_later() {
        let temporary = TempDir::new().expect("temporary marker directory should be created");
        let marker = temporary.path().join("drop-conpty-must-not-resume.marker");
        let facade = Arc::new(PausedAssignFacade::new());
        let request = program_request(
            command_shell(),
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from(format!(">\"{}\" echo resumed", marker.display())),
            ],
        );
        let start = tokio::spawn(spawn_pty_inner(
            request,
            Arc::new(OutputBuffer::new(4096)),
            facade.clone(),
            80,
            24,
        ));

        tokio::time::timeout(Duration::from_secs(5), facade.wait_until_assign_entered())
            .await
            .expect("ConPTY transaction should reach the assignment pause");
        start.abort();
        let _ = start.await;
        facade.release_assignment();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if facade.events().contains(&SpawnAuditEvent::Assigned) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("cancelled ConPTY transaction should finish cleanup");

        assert_eq!(
            facade.events(),
            vec![SpawnAuditEvent::Created, SpawnAuditEvent::Assigned]
        );
        assert!(!marker.exists(), "dropped ConPTY start executed user code");
        facade
            .wait_created_process_terminated()
            .expect("cancelled ConPTY start must leave no exact process behind");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn dropped_write_keeps_stdin_open_until_the_blocking_write_finishes() {
        let spawned = spawn_pipe_inner(
            program_request(
                command_shell(),
                &[
                    OsString::from("/D"),
                    OsString::from("/C"),
                    OsString::from("ping -n 10 127.0.0.1 >NUL"),
                ],
            ),
            Arc::new(OutputBuffer::new(4096)),
            Arc::new(SystemWin32),
        )
        .await
        .expect("stdin sink should start");
        let owner = spawned.owner.clone();
        let write_owner = owner.clone();
        let write = tokio::spawn(async move {
            write_owner
                .write(&vec![b'x'; 32 * 1024 * 1024])
                .await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        write.abort();
        let _ = write.await;
        let close_owner = owner.clone();
        let close = tokio::spawn(async move { close_owner.close_stdin().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), close)
                .await
                .is_err(),
            "close_stdin returned while the cancelled blocking write still owned stdin"
        );
        owner
            .force_kill()
            .await
            .expect("stdin sink Job should terminate");
        owner
            .wait_reaped(Instant::now() + Duration::from_secs(5))
            .await
            .expect("stdin sink should be reaped after dropped write");
        owner
            .close_stdin()
            .await
            .expect("stdin should close after the detached writer has finished");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn child_inherits_only_the_three_whitelisted_stdio_handles() {
        let sentinel = create_inheritable_pipe().expect("sentinel pipe should be created");
        clear_inheritance(sentinel.read.as_raw())
            .expect("sentinel parent read handle should not be inheritable");
        let spawned = spawn_pipe_inner(
            program_request(
                command_shell(),
                &[
                    OsString::from("/D"),
                    OsString::from("/C"),
                    OsString::from("ping -n 10 127.0.0.1 >NUL"),
                ],
            ),
            Arc::new(OutputBuffer::new(4096)),
            Arc::new(SystemWin32),
        )
        .await
        .expect("sentinel probe child should start");
        drop(sentinel.write);

        let eof = tokio::task::spawn_blocking(move || {
            let mut byte = 0_u8;
            let mut read = 0_u32;
            // SAFETY: byte is writable and the sentinel read handle remains
            // owned by this blocking task.
            let result = unsafe {
                ReadFile(
                    sentinel.read.as_raw(),
                    &mut byte,
                    1,
                    &mut read,
                    ptr::null_mut(),
                )
            };
            if result != 0 {
                return Ok(read == 0);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error().map(|code| code as u32) == Some(ERROR_BROKEN_PIPE) {
                Ok(true)
            } else {
                Err(error)
            }
        });

        let inherited = tokio::time::timeout(Duration::from_millis(250), eof)
            .await
            .map(|result| {
                result
                    .expect("sentinel reader task should not panic")
                    .expect("sentinel reader should observe EOF")
            })
            .unwrap_or(false);
        spawned
            .owner
            .force_kill()
            .await
            .expect("sentinel probe Job should terminate");
        spawned
            .owner
            .wait_reaped(Instant::now() + Duration::from_secs(5))
            .await
            .expect("sentinel probe child should be reaped");

        assert!(
            inherited,
            "an unrelated inheritable host handle reached the child despite HANDLE_LIST"
        );
    }

    #[test]
    fn crt_quoting_handles_empty_spaces_quotes_and_trailing_backslashes() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(""), r#""""#);
        assert_eq!(quoted("two words"), r#""two words""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quoted(r#"ends with \"#), r#""ends with \\""#);
        assert_eq!(
            decode_wide(&encode_command_line(
                OsStr::new(r"C:\Program Files\helper.exe"),
                &[OsString::from(r#"a\"b"#), OsString::from("")],
            )
            .expect("valid command line should encode")),
            r#""C:\Program Files\helper.exe" "a\\\"b" """#
        );
    }

    #[test]
    fn literal_powershell_kind_does_not_rewrite_script_source() {
        let script = "param($Name)\n#requires -Version 5\nWrite-Output $Name";
        let (_, args) = command_argv(&CommandSpec::Shell {
            shell: ShellKind::PowerShellLiteral,
            script: script.to_owned(),
        })
        .expect("trusted PowerShell should resolve");

        assert_eq!(args.last(), Some(&OsString::from(script)));
        assert!(!args.last().unwrap().to_string_lossy().contains("ErrorActionPreference"));
    }

    #[test]
    fn environment_overlay_is_case_insensitive_sorted_and_double_nul_terminated() {
        let inherited = vec![
            (OsString::from("Path"), OsString::from("old")),
            (OsString::from("path"), OsString::from("newer inherited")),
            (OsString::from("ZETA"), OsString::from("last")),
            (OsString::from("alpha"), OsString::from("first")),
        ];
        let overrides = BTreeMap::from([
            (OsString::from("PATH"), OsString::from("new")),
            (OsString::from("Beta"), OsString::from("middle")),
        ]);

        let block = encode_environment_from(inherited, &overrides)
            .expect("valid environment should encode");

        assert!(block.ends_with(&[0, 0]));
        assert_eq!(
            environment_entries(&block),
            vec!["alpha=first", "Beta=middle", "PATH=new", "ZETA=last"]
        );
    }

    #[test]
    fn environment_rejects_nul_in_override_key_or_value() {
        let nul = OsString::from_wide(&[b'A' as u16, 0, b'B' as u16]);
        let invalid_key = BTreeMap::from([(nul.clone(), OsString::from("value"))]);
        let invalid_value = BTreeMap::from([(OsString::from("KEY"), nul)]);

        assert!(matches!(
            encode_environment_from(Vec::new(), &invalid_key),
            Err(ProcessError::InvalidCommand { reason })
                if reason.contains("environment keys cannot contain NUL")
        ));
        assert!(matches!(
            encode_environment_from(Vec::new(), &invalid_value),
            Err(ProcessError::InvalidCommand { reason })
                if reason.contains("environment values cannot contain NUL")
        ));
    }

    #[test]
    fn environment_strips_inherited_loader_and_node_injection_variables() {
        let inherited = vec![
            (OsString::from("PATH"), OsString::from("safe")),
            (
                OsString::from("NODE_OPTIONS"),
                OsString::from("--require malicious.js"),
            ),
            (
                OsString::from("dyld_insert_libraries"),
                OsString::from("malicious.dll"),
            ),
        ];
        let block = encode_environment_from(inherited, &BTreeMap::new())
            .expect("safe inherited environment should encode");

        assert_eq!(environment_entries(&block), vec!["PATH=safe"]);
    }

    #[test]
    fn environment_rejects_case_insensitive_dangerous_overrides() {
        for key in ["NODE_OPTIONS", "node_options", "Ld_PrElOaD"] {
            let overrides = BTreeMap::from([(
                OsString::from(key),
                OsString::from("malicious"),
            )]);
            assert!(matches!(
                encode_environment_from(Vec::new(), &overrides),
                Err(ProcessError::InvalidCommand { reason })
                    if reason.contains("forbidden at process boundary")
            ));
        }
    }

    fn program_request(program: OsString, args: &[OsString]) -> NormalizedProcessRequest {
        let cwd = std::env::current_dir().expect("current directory should exist");
        NormalizedProcessRequest {
            owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
            command: CommandSpec::Program {
                program,
                args: args.to_vec(),
            },
            cwd: cwd.clone(),
            env: BTreeMap::new(),
            transport: Transport::Pipe,
            policy: ProcessPolicy::default(),
            capability: CapabilityPolicy::local_owner(cwd),
        }
    }

    fn command_shell() -> OsString {
        let system_root =
            std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
        std::path::PathBuf::from(system_root)
            .join("System32")
            .join("cmd.exe")
            .into_os_string()
    }

    fn quoted(value: &str) -> String {
        let mut encoded = Vec::new();
        append_quoted(OsStr::new(value), &mut encoded).expect("valid argument should quote");
        String::from_utf16(&encoded).expect("ASCII fixture should remain valid UTF-16")
    }

    fn decode_wide(encoded: &[u16]) -> String {
        let encoded = encoded.strip_suffix(&[0]).unwrap_or(encoded);
        String::from_utf16(encoded).expect("ASCII fixture should remain valid UTF-16")
    }

    fn environment_entries(block: &[u16]) -> Vec<String> {
        block
            .split(|unit| *unit == 0)
            .take_while(|entry| !entry.is_empty())
            .map(|entry| String::from_utf16(entry).expect("ASCII fixture should remain UTF-16"))
            .collect()
    }

    struct PausedAssignFacade {
        audit: AuditFacade,
        entered: tokio::sync::Notify,
        release: (Mutex<bool>, std::sync::Condvar),
    }

    impl PausedAssignFacade {
        fn new() -> Self {
            Self {
                audit: AuditFacade::successful(),
                entered: tokio::sync::Notify::new(),
                release: (Mutex::new(false), std::sync::Condvar::new()),
            }
        }

        async fn wait_until_assign_entered(&self) {
            self.entered.notified().await;
        }

        fn release_assignment(&self) {
            let (released, condition) = &self.release;
            let mut released = match released.lock() {
                Ok(released) => released,
                Err(poisoned) => poisoned.into_inner(),
            };
            *released = true;
            condition.notify_all();
        }

        fn events(&self) -> Vec<SpawnAuditEvent> {
            self.audit.events()
        }

        fn wait_created_process_terminated(&self) -> io::Result<()> {
            self.audit.wait_created_process_terminated()
        }
    }

    impl Win32Facade for PausedAssignFacade {
        fn process_created(
            &self,
            pid: u32,
            process: HANDLE,
            thread: HANDLE,
        ) -> io::Result<()> {
            self.audit.process_created(pid, process, thread)
        }

        fn assign_process_to_job(&self, _job: HANDLE, _process: HANDLE) -> io::Result<()> {
            self.entered.notify_one();
            let (released, condition) = &self.release;
            let mut released = released
                .lock()
                .map_err(|_| io::Error::other("assignment pause mutex is poisoned"))?;
            while !*released {
                released = condition
                    .wait(released)
                    .map_err(|_| io::Error::other("assignment pause mutex is poisoned"))?;
            }
            self.audit
                .events
                .lock()
                .map_err(|_| io::Error::other("spawn audit event mutex is poisoned"))?
                .push(SpawnAuditEvent::Assigned);
            Ok(())
        }

        fn resume_thread(&self, thread: HANDLE) -> io::Result<u32> {
            self.audit.resume_thread(thread)
        }
    }
}
