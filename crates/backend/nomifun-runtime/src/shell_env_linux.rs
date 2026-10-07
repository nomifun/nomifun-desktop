//! Bounded Linux startup-shell probing; no reader thread may outlive startup.

use std::io::{self, ErrorKind};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

struct ProbeChild {
    process: ManagedChildProcess,
    runtime: tokio::runtime::Runtime,
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        // The shared owner reaps the direct shell and proves its whole process
        // tree empty, including startup-file descendants that retain stdout.
        let _ = self.runtime.block_on(self.process.shutdown());
    }
}

pub(super) fn run(shell: &str, home_override: Option<&Path>, timeout: Duration) -> Option<String> {
    let start = Instant::now();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let mut builder = ChildProcessBuilder::new(shell);
    builder
        .args(["-i", "-l", "-c", super::PATH_PROBE_SNIPPET])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(home) = home_override {
        builder.env("HOME", home).env_remove("ZDOTDIR");
    }
    let process = {
        let _runtime = runtime.enter();
        builder.spawn_managed().ok()?
    };
    let mut child = ProbeChild { process, runtime };
    let stdout = child.process.child_mut().stdout.take()?;
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
        let drained = match read_nonblocking(stdout.as_raw_fd(), &mut buffer) {
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
            status = child.process.child_mut().try_wait().ok()?;
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

fn read_nonblocking(fd: std::os::fd::RawFd, buffer: &mut [u8]) -> io::Result<usize> {
    let length = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
    if length < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(length as usize)
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
                "#!/bin/sh\n/bin/sleep 2 &\nprintf '%s' '{}/synthetic/bin{}'\n",
                super::super::PATH_PROBE_BEGIN,
                super::super::PATH_PROBE_END
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = Instant::now();
        let result = super::super::run_login_shell_path(shell.to_str().unwrap(), None);
        assert_eq!(result.as_deref(), Some("/synthetic/bin"));
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
                "#!/bin/sh\nprintf '%s' '{}'\nprintf '%s' '{}/synthetic/bin{}'\n",
                "x".repeat(super::MAX_OUTPUT_BYTES + 1),
                super::super::PATH_PROBE_BEGIN,
                super::super::PATH_PROBE_END
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
