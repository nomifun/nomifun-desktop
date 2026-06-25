//! Windows Job Object–based child-process-tree cleanup.
//!
//! `kill_on_drop(true)` only terminates the *direct* child, and the explicit
//! `taskkill /T` teardown in [`crate::spawn`] only runs when our code gets a
//! chance to run. Neither helps when this process is force-killed — which is
//! exactly what `tauri dev` does on rebuild/Ctrl+C — so descendant trees like
//! `bunx → codex-acp → MCP stdio bridges` survive as orphans. The bridges are
//! this very executable (`nomifun-desktop.exe mcp-*-stdio`), so a leftover
//! tree keeps the binary locked and the next build dies with os error 5.
//!
//! A Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is the OS-level fix:
//! every process assigned to the job — and all descendants, which inherit
//! membership automatically — is terminated by the kernel when the last job
//! handle closes. The process-global job handle lives for the lifetime of
//! this process and is closed by the OS on process death *of any kind*,
//! including TerminateProcess.
//!
//! Residual window: membership is granted by `AssignProcessToJobObject`
//! *after* CreateProcess returns, so descendants the child manages to create
//! in those few microseconds — or anything it spawned if it exits before the
//! assignment — land outside the job. Closing it would need
//! CREATE_SUSPENDED → assign → ResumeThread, which `tokio::process` does not
//! expose; real CLI children spend far longer in loader/runtime init than
//! the window lasts, so this is accepted.

use std::io;
use std::os::windows::io::RawHandle;
use std::sync::OnceLock;

use tokio::process::Child;
use tracing::warn;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
};

/// An owned kill-on-close Job Object. Dropping it (closing the last handle)
/// makes the kernel terminate every process still assigned to it.
pub struct CleanupJob {
    handle: *mut core::ffi::c_void,
}

// SAFETY: the wrapped value is an opaque kernel handle; the Win32 job APIs
// called on it are documented thread-safe.
unsafe impl Send for CleanupJob {}
unsafe impl Sync for CleanupJob {}

impl CleanupJob {
    /// Create an anonymous job object configured with KILL_ON_JOB_CLOSE.
    pub fn new() -> io::Result<Self> {
        // SAFETY: plain Win32 calls. The handle is checked before use and
        // closed on every early-exit path.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let err = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(err);
            }

            Ok(Self { handle })
        }
    }

    /// Assign a live process to this job. Descendants spawned by the process
    /// afterwards inherit membership automatically.
    pub fn assign_raw(&self, process: RawHandle) -> io::Result<()> {
        // SAFETY: `self.handle` is live for `'self`; `process` is a live
        // process handle owned by the caller for the duration of the call.
        let ok = unsafe { AssignProcessToJobObject(self.handle, process.cast()) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    #[cfg(test)]
    pub(crate) fn raw(&self) -> *mut core::ffi::c_void {
        self.handle
    }
}

impl Drop for CleanupJob {
    fn drop(&mut self) {
        // SAFETY: `handle` is owned by `self` and closed exactly once. With
        // KILL_ON_JOB_CLOSE this terminates all processes still in the job.
        unsafe { CloseHandle(self.handle) };
    }
}

/// The process-global cleanup job. Created lazily on first spawn; lives until
/// this process dies, at which point the OS closes the handle and reaps every
/// assigned child tree. `None` if creation failed (we degrade to the existing
/// kill_on_drop / taskkill behaviour rather than refusing to spawn).
pub(crate) fn global_cleanup_job() -> Option<&'static CleanupJob> {
    static JOB: OnceLock<Option<CleanupJob>> = OnceLock::new();
    JOB.get_or_init(|| match CleanupJob::new() {
        Ok(job) => Some(job),
        Err(e) => {
            warn!(
                error = %e,
                "Failed to create cleanup job object; child process trees will leak if this process is force-killed"
            );
            None
        }
    })
    .as_ref()
}

/// Best-effort: put `child` into the global cleanup job. Failure is logged,
/// never fatal — the child still runs, we just lose the force-kill safety net.
pub(crate) fn assign_to_cleanup_job(child: &Child) {
    let Some(job) = global_cleanup_job() else { return };
    // `raw_handle` is `None` once the child has already been reaped — nothing
    // left to clean up in that case.
    let Some(raw) = child.raw_handle() else { return };
    if let Err(e) = job.assign_raw(raw) {
        warn!(pid = ?child.id(), error = %e, "Failed to assign child process to cleanup job");
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// `true` while the OS still has a live (not yet terminated) process with
    /// this pid. `tasklist /FI` prints an INFO line and nothing else when no
    /// task matches.
    fn pid_alive(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("tasklist should run");
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }

    fn wait_until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if check() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[tokio::test]
    async fn dropping_job_kills_child_and_grandchild() {
        // A plain path inside a temp dir — NOT NamedTempFile, whose open
        // handle would block PowerShell's Set-Content under Windows share
        // semantics.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild-pid.txt");

        // Leader powershell spawns a grandchild powershell, records its pid
        // into the marker file, then blocks. Mirrors the real-world shape:
        // Builder child (ACP CLI) spawning its own descendants. The marker
        // path travels via env var, not string interpolation — temp paths
        // contain the user profile, where an apostrophe would break a PS
        // single-quoted literal.
        let script = "$p = Start-Process powershell -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 120' \
             -PassThru -WindowStyle Hidden; \
             Set-Content -Path $env:NOMI_JOB_TEST_MARKER -Value $p.Id; Start-Sleep -Seconds 120";
        let mut leader = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .env("NOMI_JOB_TEST_MARKER", &marker)
            .spawn()
            .expect("spawn leader powershell");

        let job = CleanupJob::new().expect("create job");
        job.assign_raw(leader.raw_handle().expect("leader handle"))
            .expect("assign leader");

        // Wait for the grandchild pid to land in the marker file.
        assert!(
            wait_until(Duration::from_secs(20), || {
                std::fs::read_to_string(&marker)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            }),
            "grandchild pid marker should appear"
        );
        let grandchild_pid: u32 = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .expect("marker should contain a pid");
        assert!(pid_alive(grandchild_pid), "grandchild should be running");

        // Closing the last job handle must reap the whole tree — this is the
        // force-kill safety net (the OS does this even when no userland
        // cleanup code runs).
        drop(job);

        assert!(
            wait_until(Duration::from_secs(5), || leader.try_wait().ok().flatten().is_some()),
            "leader should be terminated by job close"
        );
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(grandchild_pid)),
            "grandchild pid={grandchild_pid} should be terminated by job close"
        );
    }

    #[tokio::test]
    async fn assigned_child_runs_to_completion_normally() {
        // The brief sleep keeps the child alive across the assign call — a
        // bare `exit 0` could finish first on a loaded machine, and
        // AssignProcessToJobObject fails with ACCESS_DENIED on a terminated
        // process.
        let mut child = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Milliseconds 500; exit 0"])
            .spawn()
            .expect("spawn powershell");

        let job = CleanupJob::new().expect("create job");
        job.assign_raw(child.raw_handle().expect("child handle"))
            .expect("assign child");

        let status = child.wait().await.expect("wait child");
        assert!(status.success(), "job membership must not disturb a normal run");
    }

    #[tokio::test]
    async fn global_job_is_created_once_and_assign_helper_is_silent() {
        let job1 = global_cleanup_job().expect("global job should create on a normal system") as *const _;
        let job2 = global_cleanup_job().expect("second call returns the same job") as *const _;
        assert_eq!(job1, job2, "global job must be a singleton");

        let child = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "exit 0"])
            .spawn()
            .expect("spawn powershell");
        // Must not panic / error loudly.
        assign_to_cleanup_job(&child);
    }
}
