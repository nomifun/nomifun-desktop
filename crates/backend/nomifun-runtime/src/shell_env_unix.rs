//! Bounded Unix shell probing, invoked in an isolated startup helper.

use std::io::{self, ErrorKind};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};

pub(super) const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

// Test-only diagnostics contain fixed branch labels, elapsed times and numeric
// counters. Never include the shell, its arguments, environment or captured PATH.
#[cfg(test)]
struct ProbeTestTrace {
    state: std::rc::Rc<std::cell::RefCell<ProbeTestTraceState>>,
}

#[cfg(test)]
struct ProbeTestTraceState {
    start: Instant,
    timeout_ms: u128,
    // runtime, spawn, read-loop start, first bytes, child exit, parse finish,
    // read-loop finish, shutdown start/end, Drop shutdown start/end.
    times_us: [Option<u128>; 11],
    bytes: usize,
    loop_reason: &'static str,
    result: &'static str,
    exit: Option<(Option<i32>, Option<i32>)>,
    error: Option<(ErrorKind, Option<i32>)>,
    shutdown_error: Option<(ErrorKind, Option<i32>)>,
    drop_shutdown_error: Option<(ErrorKind, Option<i32>)>,
    // lifecycle: admitted, retained jobs, pending, quarantined, blocking active,
    // submitted, completed, panics, overflow retained.
    // relay: retained, queued, active, delayed, submitted, completed, retries,
    // platform handoffs. Snapshots: start, read-loop finish, first shutdown end,
    // and final resource Drop. All values are numeric; no host metadata is read.
    metrics: [Option<([u64; 9], [u64; 8])>; 4],
}

#[cfg(test)]
fn probe_test_metrics() -> ([u64; 9], [u64; 8]) {
    let platform = nomi_process_runtime::platform_lifecycle_metrics();
    let relay = nomi_process_runtime::managed_child_cleanup_metrics();
    (
        [platform.admitted as u64, platform.retained_jobs as u64,
            platform.pending as u64, platform.quarantined as u64,
            platform.blocking_active as u64, platform.submitted,
            platform.completed, platform.panics, platform.overflow_retained],
        [relay.retained as u64, relay.queued as u64, relay.active as u64,
            relay.delayed as u64, relay.submitted, relay.completed,
            relay.retries, relay.platform_handoffs],
    )
}

#[cfg(test)]
impl ProbeTestTrace {
    fn new(start: Instant, timeout: Duration) -> Self {
        let mut metrics = [None; 4];
        metrics[0] = Some(probe_test_metrics());
        Self { state: std::rc::Rc::new(std::cell::RefCell::new(ProbeTestTraceState {
            start, timeout_ms: timeout.as_millis(), times_us: [None; 11],
            bytes: 0, loop_reason: "not-started", result: "none-early",
            exit: None, error: None, shutdown_error: None,
            drop_shutdown_error: None, metrics,
        })) }
    }

    fn mark(&self, index: usize) {
        let mut state = self.state.borrow_mut();
        state.times_us[index] = Some(state.start.elapsed().as_micros());
    }

    fn fail(&self, reason: &'static str, error: Option<&io::Error>) {
        let mut state = self.state.borrow_mut();
        state.loop_reason = reason;
        state.error = error.map(|error| (error.kind(), error.raw_os_error()));
    }

    fn read(&self, bytes: usize) {
        let mut state = self.state.borrow_mut();
        if state.times_us[3].is_none() {
            state.times_us[3] = Some(state.start.elapsed().as_micros());
        }
        state.bytes = state.bytes.saturating_add(bytes);
    }

    fn exited(&self, status: std::process::ExitStatus) {
        use std::os::unix::process::ExitStatusExt;
        self.mark(4);
        self.state.borrow_mut().exit = Some((status.code(), status.signal()));
    }

    fn finish_loop(&self, result: &Option<String>) {
        self.mark(6);
        let mut state = self.state.borrow_mut();
        state.result = if result.is_some() { "some-before-cleanup" } else { "none-loop" };
        state.metrics[1] = Some(probe_test_metrics());
    }

    fn finish_shutdown(&self, result: &io::Result<()>, parsed: bool) {
        self.mark(8);
        let mut state = self.state.borrow_mut();
        state.shutdown_error = result.as_ref().err()
            .map(|error| (error.kind(), error.raw_os_error()));
        state.result = if result.is_err() { "none-cleanup" }
            else if parsed { "some" } else { "none-loop" };
        state.metrics[2] = Some(probe_test_metrics());
    }
}

#[cfg(test)]
impl Drop for ProbeTestTrace {
    fn drop(&mut self) {
        // This guard was declared before the runtime and managed child, so this
        // single line includes their Drops, including a second shutdown attempt.
        let mut state = self.state.borrow_mut();
        state.metrics[3] = Some(probe_test_metrics());
        eprintln!("[shell-probe-test] thread={:?} timeout_ms={} times_us={:?} total_us={} bytes={} loop={} result={} exit={:?} error={:?} shutdown_error={:?} drop_shutdown_error={:?} metrics={:?}",
            std::thread::current().id(), state.timeout_ms, state.times_us,
            state.start.elapsed().as_micros(), state.bytes, state.loop_reason,
            state.result, state.exit, state.error, state.shutdown_error,
            state.drop_shutdown_error, state.metrics);
    }
}

struct ProbeChild {
    process: ManagedChildProcess,
    runtime: tokio::runtime::Runtime,
    #[cfg(test)]
    trace: std::rc::Rc<std::cell::RefCell<ProbeTestTraceState>>,
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        // The shared owner reaps the direct shell and proves its whole process
        // tree empty, including startup-file descendants that retain stdout.
        #[cfg(test)]
        {
            let mut state = self.trace.borrow_mut();
            state.times_us[9] = Some(state.start.elapsed().as_micros());
        }
        let _shutdown_result = self.runtime.block_on(self.process.shutdown());
        #[cfg(test)]
        {
            let mut state = self.trace.borrow_mut();
            state.times_us[10] = Some(state.start.elapsed().as_micros());
            state.drop_shutdown_error = _shutdown_result.as_ref().err()
                .map(|error| (error.kind(), error.raw_os_error()));
        }
    }
}

pub(super) fn run(shell: &str, home_override: Option<&Path>, timeout: Duration) -> Option<String> {
    let start = Instant::now();
    #[cfg(test)]
    let trace = ProbeTestTrace::new(start, timeout);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .inspect_err(|_error| {
            #[cfg(test)]
            trace.fail("runtime-create", Some(_error));
        })
        .ok()?;
    #[cfg(test)]
    trace.mark(0);
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
        builder.spawn_managed().inspect_err(|_error| {
            #[cfg(test)]
            trace.fail("spawn", Some(_error));
        }).ok()?
    };
    #[cfg(test)]
    trace.mark(1);
    let mut child = ProbeChild { process, runtime,
        #[cfg(test)]
        trace: std::rc::Rc::clone(&trace.state),
    };
    let result = (|| {
    #[cfg(test)]
    {
        trace.mark(2);
        if child.process.child().stdout.is_none() {
            trace.fail("stdout-missing", None);
        }
    }
    let stdout = child.process.child_mut().stdout.take()?;
    // The pipe is owned exclusively here. Nonblocking reads let the one
    // startup thread enforce the deadline even if descendants retain stdout.
    let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        #[cfg(test)]
        trace.fail("fcntl", Some(&io::Error::last_os_error()));
        return None;
    }
    let mut output = Vec::new();
    let mut status = None;
    let mut buffer = [0; 8192];
    loop {
        if start.elapsed() >= timeout {
            #[cfg(test)]
            trace.fail("read-loop-timeout", None);
            tracing::warn!("Unix login shell PATH probe timed out");
            return None;
        }
        let drained = match read_nonblocking(stdout.as_raw_fd(), &mut buffer) {
            Ok(0) => true,
            Ok(length) => {
                #[cfg(test)]
                trace.read(length);
                if output.len() + length > MAX_OUTPUT_BYTES {
                    #[cfg(test)]
                    trace.fail("output-limit", None);
                    tracing::warn!("Unix login shell PATH probe exceeded output limit");
                    return None;
                }
                output.extend_from_slice(&buffer[..length]);
                false
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => true,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_error) => {
                #[cfg(test)]
                trace.fail("read", Some(&_error));
                return None;
            }
        };
        if status.is_none() {
            status = child.process.child_mut().try_wait().inspect_err(|_error| {
                #[cfg(test)]
                trace.fail("try-wait", Some(_error));
            }).ok()?;
            #[cfg(test)]
            if let Some(status) = status {
                trace.exited(status);
            }
        }
        if let Some(status) = status {
            if !status.success() {
                #[cfg(test)]
                trace.fail("exit-status", None);
                return None;
            }
            // Drain bytes already available, but never wait for a descendant
            // to close the pipe after the actual probe command has exited.
            if drained {
                let parsed = super::extract_probe_path(std::str::from_utf8(&output)
                    .inspect_err(|_error| {
                        #[cfg(test)]
                        trace.fail("utf8", None);
                    }).ok()?);
                #[cfg(test)]
                {
                    trace.mark(5);
                    trace.fail(if parsed.is_some() { "parsed" } else { "path-extract-none" }, None);
                }
                return parsed;
            }
        }
        if drained {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    })();
    #[cfg(test)]
    trace.finish_loop(&result);
    // A parsed PATH is only admissible after the exact managed owner proves
    // the whole shell tree empty. Returning it before this shutdown succeeds
    // would let the outer helper exit zero while destroying a pending relay.
    #[cfg(test)]
    trace.mark(7);
    let shutdown_result = child.runtime.block_on(child.process.shutdown());
    #[cfg(test)]
    trace.finish_shutdown(&shutdown_result, result.is_some());
    if let Err(error) = shutdown_result {
        tracing::warn!(%error, "login shell PATH probe cleanup was not proven");
        return None;
    }
    result
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

    #[cfg(target_os = "linux")]
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
