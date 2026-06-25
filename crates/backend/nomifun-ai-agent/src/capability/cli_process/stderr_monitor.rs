use nomifun_common::AppError;
#[cfg(any(unix, windows))]
use tracing::{debug, error};

/// Force-kill a process by PID, plus any descendants.
///
/// Uses platform-native shell commands:
/// * Unix: `kill -9 -<pid>` to target the spawned process group
/// * Windows: `taskkill /F /T /PID <pid>` (`/T` walks the process tree —
///   the ACP CLI typically spawns a node/bun child that must die with it)
///
/// If the process has already exited, this is a no-op.
pub(super) fn force_kill(pid: u32, process_group_id: Option<u32>) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::io;

        fn kill_pid(target_pid: u32) -> Result<(), AppError> {
            let rc = unsafe { libc::kill(target_pid as i32, libc::SIGKILL) };
            if rc == 0 {
                debug!(pid = target_pid, "Direct SIGKILL sent successfully");
                return Ok(());
            }

            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                debug!(pid = target_pid, "Process already exited before SIGKILL");
                Ok(())
            } else {
                error!(pid = target_pid, error = %err, "Direct SIGKILL failed");
                Err(AppError::Internal(format!(
                    "Failed to kill process {target_pid}: {err}"
                )))
            }
        }

        if let Some(group_id) = process_group_id.filter(|group_id| *group_id > 1) {
            let rc = unsafe { libc::kill(-(group_id as i32), libc::SIGKILL) };
            if rc == 0 {
                debug!(pid, process_group = group_id, "SIGKILL sent successfully");
                return Ok(());
            }

            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                debug!(
                    pid,
                    process_group = group_id,
                    "Process group already exited before SIGKILL"
                );
                return kill_pid(pid);
            }

            error!(pid, process_group = group_id, error = %err, "Failed to send SIGKILL to process group");
            return Err(AppError::Internal(format!(
                "Failed to kill process group {group_id}: {err}"
            )));
        }

        kill_pid(pid)
    }
    #[cfg(windows)]
    {
        // `taskkill` exit codes:
        //   0   — process killed
        //   128 — "not found" (already exited): treat as success, identical to
        //         the unix branch's behaviour
        //   other — unexpected; surface as Internal so callers can log
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW (0x0800_0000): don't flash a console when force-killing.
        let result = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000)
            .output();

        match result {
            Ok(output) if output.status.success() => {
                debug!(pid, "taskkill /F /T succeeded");
                Ok(())
            }
            Ok(output) if output.status.code() == Some(128) => {
                debug!(pid, "Process already exited before taskkill");
                Ok(())
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output.status.code();
                error!(pid, ?code, %stderr, "taskkill returned unexpected status");
                Err(AppError::Internal(format!(
                    "taskkill failed for pid {pid} (exit {code:?}): {stderr}"
                )))
            }
            Err(e) => {
                error!(pid, error = %e, "Failed to execute taskkill");
                Err(AppError::Internal(format!("Failed to kill process {pid}: {e}")))
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(AppError::Internal(format!(
            "Force kill not supported on this platform for pid {pid}"
        )))
    }
}

#[cfg(test)]
mod force_kill_tests {
    use super::force_kill;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// Spawn a long-running OS process for the host platform.
    /// Returns the [`std::process::Child`] so the test can clean up if needed.
    fn spawn_blocker() -> std::process::Child {
        if cfg!(windows) {
            // PowerShell sleeps without using up CPU and is shipped with every
            // Windows runner image.
            Command::new("powershell")
                .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
                .spawn()
                .expect("spawn powershell sleep")
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 60"]);
            #[cfg(unix)]
            command.process_group(0);
            command.spawn().expect("spawn sleep")
        }
    }

    /// Wait up to `timeout` for the OS to reap the process; `try_wait` returns
    /// `Ok(Some(_))` once exited.
    fn wait_for_exit(child: &mut std::process::Child, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => return false,
            }
        }
    }

    #[test]
    fn force_kill_terminates_running_process() {
        let mut child = spawn_blocker();
        let pid = child.id();

        force_kill(pid, Some(pid)).expect("force_kill should succeed for live pid");

        assert!(
            wait_for_exit(&mut child, Duration::from_secs(5)),
            "process pid={pid} should exit after force_kill",
        );
    }

    #[test]
    fn force_kill_already_exited_pid_is_ok() {
        let mut child = spawn_blocker();
        let pid = child.id();
        // Reap it ourselves so the kernel removes the entry.
        let _ = child.kill();
        let _ = child.wait();

        // The pid may have been recycled by the OS in theory, but the practical
        // expectation is "kill returns success when nothing matches" — both the
        // unix `kill` non-zero exit and Windows `taskkill` rc=128 are mapped to
        // Ok in `force_kill`, so this should not produce an error.
        force_kill(pid, Some(pid)).expect("force_kill on dead pid must not error");
    }

    #[cfg(unix)]
    fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !is_pid_alive(pid) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(unix)]
    fn is_pid_alive(pid: u32) -> bool {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }
        !matches!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH))
    }

    #[cfg(unix)]
    #[test]
    fn force_kill_uses_cached_group_when_leader_has_exited() {
        use std::fs;
        use std::process::Stdio;

        let marker = tempfile::NamedTempFile::new().unwrap();
        let marker_path = marker.path().to_string_lossy().into_owned();

        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
                "cached-group-cleanup",
                marker_path.as_str(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);

        let mut leader = command.spawn().expect("spawn shell with background child");
        let leader_pid = leader.id();

        assert!(
            wait_for_exit(&mut leader, Duration::from_secs(5)),
            "leader pid={leader_pid} should exit promptly",
        );

        let child_pid: u32 = fs::read_to_string(marker.path())
            .expect("background child pid marker should exist")
            .trim()
            .parse()
            .expect("background child pid should be numeric");

        assert!(
            is_pid_alive(child_pid),
            "background child pid={child_pid} should still be alive"
        );

        force_kill(leader_pid, Some(leader_pid)).expect("force_kill should use cached process group id");

        assert!(
            wait_for_pid_exit(child_pid, Duration::from_secs(5)),
            "background child pid={child_pid} should exit after group kill",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::CliAgentProcess;
    use super::super::tests::simple_script_config;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn stderr_captured_in_buffer() {
        let config = simple_script_config("echo 'error line 1' >&2 && echo 'error line 2' >&2");
        let proc = CliAgentProcess::spawn(config).await.unwrap();

        timeout(Duration::from_secs(5), proc.wait_for_exit()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let stderr = proc.take_stderr().await;
        assert!(stderr.contains("error line 1"), "stderr: {stderr}");
        assert!(stderr.contains("error line 2"), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn take_stderr_is_consuming() {
        let config = simple_script_config("echo 'hello' >&2");
        let proc = CliAgentProcess::spawn(config).await.unwrap();

        timeout(Duration::from_secs(5), proc.wait_for_exit()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let first = proc.take_stderr().await;
        assert!(!first.is_empty());

        let second = proc.take_stderr().await;
        assert!(second.is_empty(), "Second take should be empty");
    }

    #[tokio::test]
    async fn peek_stderr_tail_returns_last_n_lines() {
        let config = simple_script_config("for i in 1 2 3 4 5; do echo \"line $i\" >&2; done");
        let proc = CliAgentProcess::spawn(config).await.unwrap();

        timeout(Duration::from_secs(5), proc.wait_for_exit()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let tail = proc.peek_stderr_tail(3).await;
        // Last three lines, in original order.
        assert!(tail.contains("line 3"), "tail: {tail}");
        assert!(tail.contains("line 4"), "tail: {tail}");
        assert!(tail.contains("line 5"), "tail: {tail}");
        assert!(!tail.contains("line 1"), "tail must drop earliest line; got {tail}");
        assert!(!tail.contains("line 2"), "tail must drop earliest line; got {tail}");
    }

    #[tokio::test]
    async fn peek_stderr_tail_does_not_drain() {
        let config = simple_script_config("echo 'first' >&2 && echo 'second' >&2");
        let proc = CliAgentProcess::spawn(config).await.unwrap();

        timeout(Duration::from_secs(5), proc.wait_for_exit()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let peek1 = proc.peek_stderr_tail(10).await;
        let peek2 = proc.peek_stderr_tail(10).await;
        assert_eq!(peek1, peek2, "peek must be idempotent");

        let drained = proc.take_stderr().await;
        assert!(drained.contains("first"));
        assert!(drained.contains("second"));
    }

    #[tokio::test]
    async fn peek_stderr_tail_zero_returns_empty() {
        let config = simple_script_config("echo 'noise' >&2");
        let proc = CliAgentProcess::spawn(config).await.unwrap();

        timeout(Duration::from_secs(5), proc.wait_for_exit()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(proc.peek_stderr_tail(0).await, "");
    }
}
