//! Bounded Linux startup-shell probing; no reader thread may outlive startup.

use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

struct ProbeChild(Child);

impl Drop for ProbeChild {
    fn drop(&mut self) {
        // The child was spawned in its own process group. Clean up ordinary
        // startup-file descendants even if the shell has already exited.
        // This is lifecycle cleanup, not a sandbox against a setsid escape.
        unsafe {
            libc::kill(-(self.0.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run(shell: &str, home_override: Option<&Path>, timeout: Duration) -> Option<String> {
    let start = Instant::now();
    let mut command = Command::new(shell);
    command
        .args(["-i", "-l", "-c", super::PATH_PROBE_SNIPPET])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    if let Some(home) = home_override {
        command.env("HOME", home).env_remove("ZDOTDIR");
    }
    let mut child = ProbeChild(command.spawn().ok()?);
    let mut stdout = child.0.stdout.take()?;
    // The pipe is owned exclusively here. Nonblocking reads let the one
    // startup thread enforce the deadline even if descendants retain stdout.
    let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return None;
    }
    let mut output = Vec::new();
    let mut status = None;
    let mut buffer = [0; 8192];
    loop {
        if start.elapsed() >= timeout {
            tracing::warn!("Linux login shell PATH probe timed out");
            return None;
        }
        let drained = match stdout.read(&mut buffer) {
            Ok(0) => true,
            Ok(length) => {
                if output.len() + length > MAX_OUTPUT_BYTES {
                    tracing::warn!("Linux login shell PATH probe exceeded output limit");
                    return None;
                }
                output.extend_from_slice(&buffer[..length]);
                false
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => true,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        if status.is_none() {
            status = child.0.try_wait().ok()?;
        }
        if let Some(status) = status {
            if !status.success() {
                return None;
            }
            // Drain bytes already available, but never wait for a descendant
            // to close the pipe after the actual probe command has exited.
            if drained {
                return super::extract_probe_path(std::str::from_utf8(&output).ok()?);
            }
        }
        if drained {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    #[test]
    fn exited_shell_with_inherited_stdout_cannot_stall_startup() {
        let directory = tempfile::tempdir().unwrap();
        let shell = directory.path().join("shell");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\n/bin/sleep 2 &\n{}\n",
                super::super::PATH_PROBE_SNIPPET
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = Instant::now();
        let result = super::super::run_login_shell_path(shell.to_str().unwrap(), None);
        assert!(result.is_some());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "a descendant retained stdout after the shell exited: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn blocked_shell_times_out_and_noisy_shell_has_bounded_output() {
        let directory = tempfile::tempdir().unwrap();
        for body in [
            "/bin/sleep 5",
            "while :; do printf 'noisy startup output'; done",
        ] {
            let shell = directory.path().join("shell");
            std::fs::write(&shell, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
            let start = Instant::now();
            assert!(
                super::run(shell.to_str().unwrap(), None, Duration::from_millis(100)).is_none()
            );
            assert!(start.elapsed() < Duration::from_secs(1));
        }
    }

    #[test]
    fn timeout_cleans_up_the_probe_process_group() {
        let directory = tempfile::tempdir().unwrap();
        let shell = directory.path().join("shell");
        let pid_file = directory.path().join("descendant.pid");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\n/bin/sleep 5 &\nprintf '%s' \"$!\" > '{}'\nwait\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(super::run(shell.to_str().unwrap(), None, Duration::from_millis(200)).is_none());
        let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                // WSL PID 1 can retain a killed orphan briefly as a zombie.
                Ok(stat) if stat.split_whitespace().nth(2) != Some("Z") => {
                    assert!(
                        Instant::now() < deadline,
                        "probe descendant {pid} is still running"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => break,
            }
        }
    }

    #[test]
    fn output_limit_rejects_noise_even_with_a_valid_trailing_marker() {
        let directory = tempfile::tempdir().unwrap();
        let shell = directory.path().join("shell");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\n{}\n",
                "x".repeat(super::MAX_OUTPUT_BYTES + 1),
                super::super::PATH_PROBE_SNIPPET
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(super::run(shell.to_str().unwrap(), None, Duration::from_secs(2)).is_none());
    }

    #[test]
    fn large_valid_path_is_drained_without_pipe_deadlock() {
        let directory = tempfile::tempdir().unwrap();
        let shell = directory.path().join("shell");
        let path = format!("/{}", "x".repeat(128 * 1024));
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf '%s' '{}{}{}'\n",
                super::super::PATH_PROBE_BEGIN,
                path,
                super::super::PATH_PROBE_END
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            super::run(shell.to_str().unwrap(), None, Duration::from_secs(1)),
            Some(path)
        );
    }
}
