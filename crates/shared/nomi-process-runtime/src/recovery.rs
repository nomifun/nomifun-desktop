//! Cross-platform exact process identity + verified orphan termination.
//!
//! A durable registry can record which child processes an application spawned,
//! but after a restart a bare PID is not evidence: PIDs recycle. This module
//! supplies the persisted half of that proof — an [`ExactProcessIdentity`]
//! captured while the spawner still owns the live child — and the boot-time
//! half: re-proving that whatever runs under the recorded PID *is* the
//! recorded instance before touching it, then killing its tree and proving
//! absence.
//!
//! Proof obligations:
//! - [`capture_child_identity`] must be called while the child handle is
//!   owned and unreaped, so the PID cannot have been recycled.
//! - [`probe_process_identity`] returning `Ok(None)` is positive proof that
//!   no process runs under the PID at probe time. Errors are *not* proof.
//! - [`terminate_verified_orphan`] never signals a process whose live
//!   identity does not match the recorded one; a mismatch is itself proof
//!   that the recorded instance already exited.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Exact identity of one OS process instance, serializable for durable
/// registries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactProcessIdentity {
    pub pid: u32,
    /// Best-effort wall-clock start seconds (0 when the platform cannot
    /// supply it cheaply). Diagnostic only — never part of the equivalence
    /// rule, because wall-clock derivations differ across readers.
    pub start_time_epoch_seconds: u64,
    /// Platform-native creation key: Windows full creation FILETIME as u64;
    /// Linux `/proc/<pid>/stat` field 22 (start time in boot ticks); macOS
    /// `pbi_start_tvsec * 1_000_000 + pbi_start_tvusec`.
    pub platform_start_key: u64,
    /// Normalized executable path when readable at capture time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
}

/// Outcome of one verified orphan termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrphanTerminationOutcome {
    /// The recorded instance no longer runs (nothing under the PID, or the
    /// PID is owned by a different instance). Nothing was signalled.
    AlreadyDead,
    /// The recorded instance was re-verified live, its tree was killed, and
    /// absence was proven before returning.
    KilledAndProven,
}

#[cfg(windows)]
impl From<crate::platform::windows::WindowsProcessIdentity> for ExactProcessIdentity {
    fn from(identity: crate::platform::windows::WindowsProcessIdentity) -> Self {
        Self {
            pid: identity.pid,
            start_time_epoch_seconds: identity.start_time_epoch_seconds,
            platform_start_key: identity.platform_start_key,
            executable: Some(identity.executable),
        }
    }
}

/// Identity equivalence for recovery decisions: PID and platform start key
/// must match. The executable is extra defense only — it vetoes when both
/// sides carry one and they disagree; a missing side never vetoes, because
/// executable paths can be unreadable at capture or probe time while the
/// (pid, start key) pair already uniquely names one process instance.
pub fn same_recorded_process(recorded: &ExactProcessIdentity, live: &ExactProcessIdentity) -> bool {
    if recorded.pid != live.pid || recorded.platform_start_key != live.platform_start_key {
        return false;
    }
    match (&recorded.executable, &live.executable) {
        (Some(recorded_exe), Some(live_exe)) => recorded_exe == live_exe,
        _ => true,
    }
}

/// Capture the exact identity of a freshly spawned Tokio child.
///
/// Must be called while `child` is owned and unreaped: the live handle (or
/// the unreaped PID on Unix) pins the instance, so the identity read cannot
/// race a PID recycle.
pub fn capture_child_identity(
    child: &tokio::process::Child,
) -> io::Result<ExactProcessIdentity> {
    #[cfg(windows)]
    {
        crate::platform::windows::windows_child_process_identity(child).map(Into::into)
    }

    #[cfg(unix)]
    {
        let pid = child.id().ok_or_else(|| {
            io::Error::other("child process has already been reaped; identity unavailable")
        })?;
        probe_process_identity(pid)?.ok_or_else(|| {
            io::Error::other(format!(
                "spawned child {pid} was not observable during identity capture"
            ))
        })
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = child;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process identity capture is unavailable on this platform",
        ))
    }
}

/// Inspect one PID. `Ok(None)` is positive proof that no process currently
/// runs under it; `Ok(Some)` is the live process's current identity; errors
/// mean the platform could not answer and prove nothing.
pub fn probe_process_identity(pid: u32) -> io::Result<Option<ExactProcessIdentity>> {
    if pid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot probe PID 0",
        ));
    }

    #[cfg(windows)]
    {
        windows_impl::probe(pid)
    }

    #[cfg(target_os = "linux")]
    {
        linux_impl::probe(pid)
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::probe(pid)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process identity probing is unavailable on this platform",
        ))
    }
}

/// Kill the recorded process instance's group/tree, but only after re-proving
/// that whatever runs under the recorded PID *is* the recorded instance, and
/// only returning success once absence is proven. Blocking; bounded by
/// `timeout`. Callers on an async runtime should wrap it in `spawn_blocking`.
pub fn terminate_verified_orphan(
    identity: &ExactProcessIdentity,
    timeout: Duration,
) -> io::Result<OrphanTerminationOutcome> {
    let Some(live) = probe_process_identity(identity.pid)
        .map_err(|e| io::Error::new(e.kind(), format!("pre-termination probe: {e}")))?
    else {
        return Ok(OrphanTerminationOutcome::AlreadyDead);
    };
    if !same_recorded_process(identity, &live) {
        // The PID was recycled: the recorded instance provably exited. Never
        // signal the unrelated current owner.
        return Ok(OrphanTerminationOutcome::AlreadyDead);
    }

    #[cfg(windows)]
    {
        windows_impl::terminate(identity, timeout)
    }

    #[cfg(target_os = "linux")]
    {
        linux_impl::terminate(identity, timeout)
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::terminate(identity, timeout)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = timeout;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "verified orphan termination is unavailable on this platform",
        ))
    }
}

/// Poll `probe_process_identity` until the recorded instance is gone.
/// "Gone" means nothing runs under the PID, or a different instance owns it.
#[cfg_attr(windows, allow(dead_code))]
fn confirm_recorded_instance_absent(
    identity: &ExactProcessIdentity,
    deadline: Instant,
) -> io::Result<OrphanTerminationOutcome> {
    let mut backoff = Duration::from_millis(10);
    loop {
        match probe_process_identity(identity.pid)? {
            None => return Ok(OrphanTerminationOutcome::KilledAndProven),
            Some(live) if !same_recorded_process(identity, &live) => {
                return Ok(OrphanTerminationOutcome::KilledAndProven);
            }
            Some(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "recorded process {} still runs after verified termination",
                    identity.pid
                ),
            ));
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_millis(200));
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use crate::platform::windows::{
        WindowsExactProcess, WindowsRecoveryJob, probe_running_process_identity,
    };

    const ERROR_INVALID_PARAMETER: i32 = 87;

    pub(super) fn probe(pid: u32) -> io::Result<Option<ExactProcessIdentity>> {
        // A terminated-but-unreaped process object (e.g. a handle still held
        // by its parent) probes as absent: it can no longer execute.
        Ok(probe_running_process_identity(pid)?.map(Into::into))
    }

    pub(super) fn terminate(
        identity: &ExactProcessIdentity,
        timeout: Duration,
    ) -> io::Result<OrphanTerminationOutcome> {
        let process = match WindowsExactProcess::open_for_recovery(identity.pid) {
            Ok(process) => process,
            Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) => {
                return Ok(OrphanTerminationOutcome::AlreadyDead);
            }
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("open exact process for recovery: {error}"),
                ));
            }
        };
        // Re-verify through the recovery handle itself: the handle pins one
        // exact instance, so a mismatch means the recorded one is gone.
        let handle_identity: ExactProcessIdentity = process.identity().clone().into();
        if !same_recorded_process(identity, &handle_identity) {
            return Ok(OrphanTerminationOutcome::AlreadyDead);
        }

        let mut job = WindowsRecoveryJob::new_unarmed()
            .map_err(|e| io::Error::new(e.kind(), format!("create recovery job: {e}")))?;
        job.assign(&process)
            .map_err(|e| io::Error::new(e.kind(), format!("assign recovery job: {e}")))?;
        job.arm_kill_on_close()
            .map_err(|e| io::Error::new(e.kind(), format!("arm recovery job: {e}")))?;
        job.terminate_and_wait([&process], timeout)
            .map_err(|e| io::Error::new(e.kind(), format!("terminate recovery job: {e}")))?;
        Ok(OrphanTerminationOutcome::KilledAndProven)
    }
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use super::*;
    use crate::platform::linux_recovery::LinuxProcessGroupAnchor;

    pub(super) fn probe(pid: u32) -> io::Result<Option<ExactProcessIdentity>> {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error)
                if matches!(error.kind(), io::ErrorKind::NotFound)
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        // Field 22 (1-indexed) after the parenthesized comm, which may itself
        // contain spaces: split after the LAST ')'.
        let after_comm = stat
            .rfind(')')
            .map(|index| &stat[index + 1..])
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "malformed /proc stat line")
            })?;
        let platform_start_key = after_comm
            .split_ascii_whitespace()
            .nth(19)
            .and_then(|field| field.parse::<u64>().ok())
            .filter(|key| *key != 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing /proc stat starttime field",
                )
            })?;
        let executable = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
        Ok(Some(ExactProcessIdentity {
            pid,
            start_time_epoch_seconds: start_seconds_from_boot_ticks(platform_start_key),
            platform_start_key,
            executable,
        }))
    }

    /// Best-effort wall-clock start derivation; 0 when /proc/stat btime or
    /// the clock-tick rate is unavailable.
    fn start_seconds_from_boot_ticks(start_ticks: u64) -> u64 {
        let btime = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|stat| {
                stat.lines().find_map(|line| {
                    line.strip_prefix("btime ")
                        .and_then(|value| value.trim().parse::<u64>().ok())
                })
            });
        // SAFETY: sysconf is a read-only libc query.
        let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        match (btime, ticks_per_second) {
            (Some(btime), ticks) if ticks > 0 => btime + start_ticks / ticks as u64,
            _ => 0,
        }
    }

    pub(super) fn terminate(
        identity: &ExactProcessIdentity,
        timeout: Duration,
    ) -> io::Result<OrphanTerminationOutcome> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
        let mut anchor = match LinuxProcessGroupAnchor::open(identity.pid) {
            Ok(anchor) => anchor,
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                return Ok(OrphanTerminationOutcome::AlreadyDead);
            }
            Err(error) => return Err(error),
        };
        anchor.stop()?;
        // Re-verify while stopped: a recycle between the outer verification
        // and the SIGSTOP would otherwise freeze-and-kill an innocent PID.
        match probe_process_identity(identity.pid)? {
            None => return Ok(OrphanTerminationOutcome::AlreadyDead),
            Some(live) if !same_recorded_process(identity, &live) => {
                // Drop resumes the stopped imposter via SIGCONT.
                return Ok(OrphanTerminationOutcome::AlreadyDead);
            }
            Some(_) => {}
        }
        anchor.terminate_group()?;
        confirm_recorded_instance_absent(identity, deadline)
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::*;

    pub(super) fn probe(pid: u32) -> io::Result<Option<ExactProcessIdentity>> {
        let pid_t = pid as libc::pid_t;
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: info is a writable proc_bsdinfo of the advertised size.
        let written = unsafe {
            libc::proc_pidinfo(
                pid_t,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size,
            )
        };
        if written <= 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(None);
            }
            return Err(error);
        }
        if written != size || info.pbi_pid != pid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "proc_pidinfo returned an inconsistent bsdinfo record",
            ));
        }
        let platform_start_key = u64::from(info.pbi_start_tvsec)
            .checked_mul(1_000_000)
            .and_then(|micros| micros.checked_add(u64::from(info.pbi_start_tvusec)))
            .filter(|key| *key != 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "process start time is not representable",
                )
            })?;
        Ok(Some(ExactProcessIdentity {
            pid,
            start_time_epoch_seconds: u64::from(info.pbi_start_tvsec),
            platform_start_key,
            executable: executable_path(pid_t),
        }))
    }

    fn executable_path(pid: libc::pid_t) -> Option<PathBuf> {
        let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: buffer is writable for its advertised length.
        let written = unsafe {
            libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32)
        };
        if written <= 0 {
            return None;
        }
        buffer.truncate(written as usize);
        Some(PathBuf::from(String::from_utf8_lossy(&buffer).into_owned()))
    }

    pub(super) fn terminate(
        identity: &ExactProcessIdentity,
        timeout: Duration,
    ) -> io::Result<OrphanTerminationOutcome> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
        let pid = identity.pid as libc::pid_t;

        // Group-leadership proof: our contained spawns are made their own
        // process-group leaders, so the negative-PID signal below addresses
        // exactly the recorded instance's descendants. A non-leader cannot be
        // safely group-killed and fails closed.
        // SAFETY: getpgid is a read-only query.
        let pgid = unsafe { libc::getpgid(pid) };
        if pgid < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(OrphanTerminationOutcome::AlreadyDead);
            }
            return Err(error);
        }
        if pgid != pid {
            return Err(io::Error::other(
                "recorded process is not its own process-group leader; refusing group kill",
            ));
        }
        // SAFETY: getpgrp is a read-only query for the current process.
        if unsafe { libc::getpgrp() } == pid {
            return Err(io::Error::other(
                "refusing to terminate the current application process group",
            ));
        }
        // Final identity re-check immediately before the signal narrows the
        // recycle window to the syscall gap, mirroring the browser-profile
        // recovery precedent (macOS has no pidfd equivalent to close it).
        match probe_process_identity(identity.pid)? {
            None => return Ok(OrphanTerminationOutcome::AlreadyDead),
            Some(live) if !same_recorded_process(identity, &live) => {
                return Ok(OrphanTerminationOutcome::AlreadyDead);
            }
            Some(_) => {}
        }
        // SAFETY: pid was proven a live group leader distinct from our own
        // group; killpg addresses only that verified group.
        if unsafe { libc::killpg(pid, libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(OrphanTerminationOutcome::AlreadyDead);
            }
            return Err(error);
        }
        confirm_recorded_instance_absent(identity, deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    fn sleeper_command() -> tokio::process::Command {
        let mut cmd = if cfg!(windows) {
            let mut cmd = tokio::process::Command::new("ping");
            cmd.args(["-n", "60", "127.0.0.1"]);
            cmd
        } else {
            let mut cmd = tokio::process::Command::new("sleep");
            cmd.arg("60");
            cmd
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Verified group termination requires the recorded process to lead
        // its own group, matching the contained-spawn contract.
        #[cfg(unix)]
        cmd.process_group(0);
        cmd
    }

    fn spawn_sleeper() -> (tokio::process::Child, ExactProcessIdentity) {
        let child = sleeper_command().spawn().expect("spawn test sleeper");
        let identity = capture_child_identity(&child).expect("capture identity");
        (child, identity)
    }

    #[test]
    fn identity_serde_round_trips_with_and_without_executable() {
        let with_exe = ExactProcessIdentity {
            pid: 42,
            start_time_epoch_seconds: 1_722_400_000,
            platform_start_key: 133_663_000_000_000_000,
            executable: Some(PathBuf::from("C:/tools/agent.exe")),
        };
        let json = serde_json::to_string(&with_exe).unwrap();
        assert_eq!(
            serde_json::from_str::<ExactProcessIdentity>(&json).unwrap(),
            with_exe
        );

        let without_exe = ExactProcessIdentity {
            executable: None,
            ..with_exe
        };
        let json = serde_json::to_string(&without_exe).unwrap();
        assert!(!json.contains("executable"));
        assert_eq!(
            serde_json::from_str::<ExactProcessIdentity>(&json).unwrap(),
            without_exe
        );
    }

    #[test]
    fn equivalence_requires_pid_and_start_key_with_executable_as_extra_defense() {
        let base = ExactProcessIdentity {
            pid: 7,
            start_time_epoch_seconds: 1,
            platform_start_key: 99,
            executable: Some(PathBuf::from("agent")),
        };
        assert!(same_recorded_process(&base, &base.clone()));
        assert!(!same_recorded_process(
            &base,
            &ExactProcessIdentity { pid: 8, ..base.clone() }
        ));
        assert!(!same_recorded_process(
            &base,
            &ExactProcessIdentity {
                platform_start_key: 100,
                ..base.clone()
            }
        ));
        let missing_exe = ExactProcessIdentity {
            executable: None,
            ..base.clone()
        };
        assert!(same_recorded_process(&base, &missing_exe));
        assert!(same_recorded_process(&missing_exe, &base));
        assert!(!same_recorded_process(
            &base,
            &ExactProcessIdentity {
                executable: Some(PathBuf::from("other")),
                ..base.clone()
            }
        ));
    }

    #[tokio::test]
    async fn capture_probe_terminate_prove_the_full_lifecycle() {
        let (mut child, identity) = spawn_sleeper();

        let live = probe_process_identity(identity.pid)
            .unwrap()
            .expect("live child must probe as present");
        assert!(same_recorded_process(&identity, &live));

        let outcome = tokio::task::spawn_blocking({
            let identity = identity.clone();
            move || terminate_verified_orphan(&identity, Duration::from_secs(10))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(outcome, OrphanTerminationOutcome::KilledAndProven);
        assert!(probe_process_identity(identity.pid).unwrap().is_none()
            || !same_recorded_process(
                &identity,
                &probe_process_identity(identity.pid).unwrap().unwrap()
            ));

        // The child was killed externally; reap it through the Tokio handle.
        let _ = child.wait().await;
    }

    #[tokio::test]
    async fn identity_mismatch_is_already_dead_and_never_kills() {
        let (mut child, identity) = spawn_sleeper();
        let tampered = ExactProcessIdentity {
            platform_start_key: identity.platform_start_key + 1,
            ..identity.clone()
        };

        let outcome = tokio::task::spawn_blocking(move || {
            terminate_verified_orphan(&tampered, Duration::from_secs(5))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(outcome, OrphanTerminationOutcome::AlreadyDead);
        assert!(
            child.try_wait().unwrap().is_none(),
            "a mismatched identity must never kill the live pid owner"
        );

        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn dead_pid_probes_as_absent_and_terminates_as_already_dead() {
        let (mut child, identity) = spawn_sleeper();
        child.kill().await.unwrap();
        child.wait().await.unwrap();

        // The PID may be recycled by an unrelated process between kill and
        // probe; both "absent" and "different instance" prove the recorded
        // one dead.
        match probe_process_identity(identity.pid).unwrap() {
            None => {}
            Some(live) => assert!(!same_recorded_process(&identity, &live)),
        }
        let outcome = tokio::task::spawn_blocking({
            let identity = identity.clone();
            move || terminate_verified_orphan(&identity, Duration::from_secs(5))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(outcome, OrphanTerminationOutcome::AlreadyDead);
    }

    #[test]
    fn probing_pid_zero_is_an_error_not_a_proof() {
        assert!(probe_process_identity(0).is_err());
    }
}
