//! Isolate startup discovery before the host mutates its environment.
//!
//! Managed child ownership starts the permanent lifecycle poller. Running that
//! work in a forked helper keeps those threads out of the single-threaded host;
//! the host can then safely apply its startup environment after exact helper
//! reaping. This path never loads AppKit or calls the Objective-C runtime.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

const RESULT_HEADER_BYTES: usize = 12;
const RESULT_MAGIC: &[u8; 4] = b"NFS1";

/// Run bounded discovery in an isolated helper, then reap that exact child.
///
/// The callback may create a runtime and managed children. Its process owners
/// must retain their normal watchdog cleanup authority. If the helper exceeds
/// the total budget, this parent kills only its still-unreaped direct child;
/// any managed grandchildren remain owned by their already-registered
/// watchdogs. Success means the callback completed and the helper exited zero,
/// not a general proof about arbitrary processes created by the callback.
///
/// # Safety
/// Call only during single-threaded startup, before any other host thread or
/// process-runtime singleton has been created. No other thread may hold a Rust,
/// libc, loader, or Objective-C lock at fork. The callback must not initialize
/// AppKit or the Objective-C runtime. Do not call this from an async runtime or
/// a test-harness worker; use a standalone process for acceptance tests. The
/// host must not install SIGCHLD auto-reaping or a handler that reaps children:
/// this function alone owns its forked helper's exact waitpid lease.
pub unsafe fn run_isolated_startup_probe<F>(
    timeout: Duration,
    max_output_bytes: usize,
    probe: F,
) -> io::Result<Vec<u8>>
where
    F: FnOnce() -> io::Result<Vec<u8>>,
{
    if timeout.is_zero() || max_output_bytes == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "startup probe requires a nonzero budget"));
    }
    let max_frame_bytes = max_output_bytes.checked_add(RESULT_HEADER_BYTES)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "startup probe output budget is too large"))?;
    let deadline = Instant::now().checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "startup probe timeout is too large"))?;
    let mut fds = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let reader = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let writer = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    for fd in [reader.as_raw_fd(), writer.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_os = "macos")]
    super::macos_watchdog::prepare_host_for_fork();
    // SAFETY: the caller guarantees that the host has not started threads.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        drop(reader);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe));
        let code = match result {
            Ok(Ok(bytes)) if bytes.len() <= max_output_bytes => {
                let mut header = [0_u8; RESULT_HEADER_BYTES];
                header[..4].copy_from_slice(RESULT_MAGIC);
                header[4..].copy_from_slice(&(bytes.len() as u64).to_be_bytes());
                write_all(writer.as_raw_fd(), &header)
                    .and_then(|()| write_all(writer.as_raw_fd(), &bytes))
                    .map_or(70, |()| 0)
            }
            _ => 70,
        };
        // The callback's managed cleanup has completed before a successful
        // result. _exit never runs inherited host destructors or GUI hooks.
        unsafe { libc::_exit(code) };
    }
    drop(writer);
    let mut child = StartupChild { pid: Some(pid) };
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "isolated startup probe exceeded its total budget"));
        }
        let length = unsafe { libc::read(reader.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        let drained = if length > 0 {
            let length = length as usize;
            if length > max_frame_bytes.saturating_sub(output.len()) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "isolated startup probe exceeded its output bound"));
            }
            output.extend_from_slice(&buffer[..length]);
            false
        } else if length < 0 {
            let error = io::Error::last_os_error();
            match error.kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => true,
                _ => return Err(error),
            }
        } else {
            true
        };
        if let Some(status) = child.try_reap()? {
            if !libc::WIFEXITED(status) || libc::WEXITSTATUS(status) != 0 {
                return Err(io::Error::other("isolated startup probe did not complete successfully"));
            }
            // Reaping seals the helper identity. Drain only queued bytes; an
            // unrelated inherited writer cannot extend the startup budget.
            if drained {
                // Exit zero alone is not a callback-completion receipt. Require
                // the complete bounded result frame emitted only after it
                // returned successfully; early helper exit or truncation fails.
                if output.len() < RESULT_HEADER_BYTES || &output[..4] != RESULT_MAGIC {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "isolated startup probe omitted its completion receipt"));
                }
                let expected = u64::from_be_bytes(output[4..RESULT_HEADER_BYTES].try_into().unwrap());
                if expected != (output.len() - RESULT_HEADER_BYTES) as u64 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "isolated startup probe returned a truncated result"));
                }
                return Ok(output.split_off(RESULT_HEADER_BYTES));
            }
        }
        if drained {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn write_all(fd: libc::c_int, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if written > 0 {
            bytes = &bytes[written as usize..];
        } else if written < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        } else if written == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "startup result pipe did not accept output"));
        } else {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

struct StartupChild {
    /// Cleared immediately on successful waitpid. A numeric PID after reap can
    /// never become signal authority, even if draining the pipe then fails.
    pid: Option<libc::pid_t>,
}

impl StartupChild {
    fn try_reap(&mut self) -> io::Result<Option<libc::c_int>> {
        let Some(pid) = self.pid else { return Ok(Some(0)); };
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if result == pid {
                self.pid = None;
                return Ok(Some(status));
            }
            if result == 0 {
                return Ok(None);
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted { continue; }
            // Loss of parent ownership is terminal; never signal cached PID.
            if error.raw_os_error() == Some(libc::ECHILD) { self.pid = None; }
            return Err(error);
        }
    }
}

impl Drop for StartupChild {
    fn drop(&mut self) {
        let Some(pid) = self.pid.take() else { return; };
        // This exact direct child has never been reaped, so the kernel cannot
        // reuse its PID. Its managed watchdogs retain descendant ownership.
        let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(pid, &mut status, 0) };
            if result == pid { break; }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted { break; }
        }
    }
}
