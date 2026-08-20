//! Low-level PTY adapter backed by `nomi-process-runtime`.
//!
//! The runtime owns the platform-specific process-tree authority (Unix
//! watchdog/process group and Windows Job/ConPTY), so a backend crash cannot
//! leave an ordinary PTY session running against the managed workspace.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nomi_process_runtime::{
    CapabilityPolicy, CleanupReport, CommandSpec, OutputCursor, OutputObserver, OutputStream,
    ProcessHandle, ProcessOutcome, ProcessOwner, ProcessPolicy, ProcessRequest,
    ProcessSupervisor, SupervisorConfig, Transport, normalize_request,
};
#[cfg(windows)]
use nomi_process_runtime::ShellKind;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::error::TerminalError;

/// Max bytes retained for reconnect scrollback (~256 KB).
const SCROLLBACK_CAP: usize = 256 * 1024;

/// Bounded fan-out buffer for the live output stream (in chunks). A lagging
/// subscriber drops oldest chunks rather than stalling the PTY reader.
const OUTPUT_BROADCAST_CAP: usize = 512;

/// The terminal-facing interpretation of the runtime's terminal outcome.
///
/// `Lost` is deliberately separate from a normal exit. Callers must preserve
/// the durable management row and must not report a successful delete/relaunch
/// when exact process ownership or cleanup was lost.
#[derive(Debug, Clone)]
pub enum PtyExit {
    Exited(Option<i32>),
    Lost {
        message: String,
        cleanup_reaped: bool,
    },
}

type ExitCallback = Box<dyn FnOnce(PtyExit, Vec<u8>) + Send + 'static>;

/// A live PTY session owned by one process-runtime supervisor.
pub struct PtyHandle {
    supervisor: Arc<ProcessSupervisor>,
    process: ProcessHandle,
    scrollback: Arc<Mutex<Vec<u8>>>,
    dirty: Arc<AtomicBool>,
    out_tx: broadcast::Sender<Vec<u8>>,
    pid: Option<u32>,
    exit_callback: Mutex<Option<ExitCallback>>,
    exit_monitor_started: AtomicBool,
    quarantined: AtomicBool,
    #[cfg(test)]
    fail_next_kill: AtomicBool,
    epoch: u64,
}

/// Parameters for spawning a PTY.
pub struct SpawnParams {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub cols: u16,
    pub rows: u16,
}

impl PtyHandle {
    /// Spawn a child under the runtime's platform ownership transaction.
    ///
    /// Output observation is installed before user code can run, so live
    /// callbacks do not depend on the runtime's bounded replay ring. The exit
    /// monitor is intentionally armed separately via [`Self::activate`]:
    /// `TerminalService` first inserts the returned handle into its epoch map,
    /// closing the quick-exit-before-registration race.
    pub async fn spawn<FOut, FExit>(
        params: SpawnParams,
        epoch: u64,
        on_output: FOut,
        on_exit: FExit,
    ) -> Result<Arc<Self>, TerminalError>
    where
        FOut: Fn(Vec<u8>) + Send + 'static,
        FExit: FnOnce(PtyExit, Vec<u8>) + Send + 'static,
    {
        let session_cwd = std::env::current_dir()
            .map_err(|error| TerminalError::Spawn(format!("resolve current directory: {error}")))?;
        let requested_cwd = if params.cwd.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(&params.cwd)
        };
        let capability_root = if requested_cwd.is_absolute() {
            requested_cwd.clone()
        } else {
            session_cwd.join(&requested_cwd)
        };

        let scrollback = Arc::new(Mutex::new(Vec::<u8>::new()));
        let dirty = Arc::new(AtomicBool::new(false));
        let (out_tx, _) = broadcast::channel::<Vec<u8>>(OUTPUT_BROADCAST_CAP);
        let output_callback =
            Arc::new(Mutex::new(Box::new(on_output) as Box<dyn Fn(Vec<u8>) + Send>));

        let observer_scrollback = Arc::clone(&scrollback);
        let observer_dirty = Arc::clone(&dirty);
        let observer_tx = out_tx.clone();
        let observer_callback = Arc::clone(&output_callback);
        let observer: OutputObserver = Arc::new(move |stream, bytes| {
            if stream != OutputStream::Pty || bytes.is_empty() {
                return;
            }
            let chunk = bytes.to_vec();
            append_scrollback(&observer_scrollback, &chunk);
            observer_dirty.store(true, Ordering::Relaxed);
            let _ = observer_tx.send(chunk.clone());
            let callback = observer_callback
                .lock()
                .expect("PTY output callback lock is poisoned");
            callback(chunk);
        });

        let mut policy = ProcessPolicy::default();
        // The runtime's bounded replay ring is never read for terminals:
        // PtyHandle keeps its own lossless scrollback (fed by the observer
        // above), and exit/kill outcomes only consume code + cleanup
        // evidence. A zero cap avoids retaining a second ~256 KiB copy of
        // every live terminal's output.
        policy.output_limit_bytes = 0;
        // Desktop terminals live until the user closes/relaunches them or the
        // backend shuts down. A lease would race the reaper after long sleep.
        policy.expire_on_idle = false;

        let command = command_with_locale_precedence(params.program, params.args, &params.env);
        let request = ProcessRequest {
            owner: ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7()),
            command,
            cwd: requested_cwd,
            env: params
                .env
                .into_iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value)))
                .collect::<BTreeMap<_, _>>(),
            transport: Transport::Pty {
                cols: params.cols,
                rows: params.rows,
            },
            policy,
            capability: CapabilityPolicy::local_owner(capability_root),
        };
        let request = normalize_request(request, &session_cwd)
            .map_err(|error| TerminalError::Spawn(error.to_string()))?;
        let supervisor = ProcessSupervisor::new(SupervisorConfig {
            max_sessions: 1,
            ..SupervisorConfig::default()
        });
        let process = supervisor
            .start_with_output_observer(request, observer)
            .await
            .map_err(|error| TerminalError::Spawn(error.to_string()))?;
        // There is deliberately no await between the committed runtime start
        // and assembling the adapter handle, so cancellation cannot strand an
        // owned process after `start` returns.
        let pid = Some(process.pid);

        Ok(Arc::new(Self {
            supervisor,
            process,
            scrollback,
            dirty,
            out_tx,
            pid,
            exit_callback: Mutex::new(Some(Box::new(on_exit))),
            exit_monitor_started: AtomicBool::new(false),
            quarantined: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_kill: AtomicBool::new(false),
            epoch,
        }))
    }

    /// Arm exactly one terminal-outcome monitor after the service has published
    /// this handle as the authoritative live epoch.
    pub fn activate(self: &Arc<Self>) {
        if self.exit_monitor_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = Arc::clone(self);
        tokio::spawn(async move {
            handle.monitor_exit().await;
        });
    }

    async fn monitor_exit(self: Arc<Self>) {
        let outcome = loop {
            match self
                .supervisor
                .poll(
                    &self.process.owner,
                    &self.process.session_id,
                    OutputCursor::START,
                    Instant::now() + Duration::from_secs(24 * 60 * 60),
                )
                .await
            {
                Ok(nomi_process_runtime::PollResult::Running { .. }) => continue,
                Ok(nomi_process_runtime::PollResult::Finished(outcome)) => break outcome,
                Err(error) => {
                    let exit = PtyExit::Lost {
                        message: format!("PTY outcome monitor lost runtime ownership: {error}"),
                        cleanup_reaped: false,
                    };
                    self.mark_quarantined();
                    self.fire_exit(exit);
                    return;
                }
            }
        };

        let exit = exit_from_outcome(&outcome);
        if matches!(exit, PtyExit::Lost { .. }) {
            self.mark_quarantined();
        }
        self.fire_exit(exit);
    }

    fn fire_exit(&self, exit: PtyExit) {
        let callback = self
            .exit_callback
            .lock()
            .expect("PTY exit callback lock is poisoned")
            .take();
        if let Some(callback) = callback {
            callback(exit, self.scrollback());
        }
    }

    /// Write bytes to the PTY.
    pub async fn write(&self, bytes: &[u8]) -> Result<(), TerminalError> {
        self.ensure_not_quarantined()?;
        self.supervisor
            .write(&self.process.owner, &self.process.session_id, bytes)
            .await
            .map_err(|error| TerminalError::Spawn(format!("write PTY: {error}")))
    }

    /// Resize the PTY window.
    pub async fn resize(&self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        self.ensure_not_quarantined()?;
        self.supervisor
            .resize(&self.process.owner, &self.process.session_id, cols, rows)
            .await
            .map_err(|error| TerminalError::Spawn(format!("resize PTY: {error}")))
    }

    /// Immediately force-kill and reap the owned PTY session/process group.
    ///
    /// `Ok` is returned only for a non-`Lost` outcome whose cleanup evidence
    /// proves the platform authority empty.
    pub async fn kill(&self) -> Result<(), TerminalError> {
        #[cfg(test)]
        if self.fail_next_kill.swap(false, Ordering::SeqCst) {
            return Err(TerminalError::Spawn(
                "injected PTY process-tree kill failure".to_owned(),
            ));
        }

        let outcome = self
            .supervisor
            .force_kill(&self.process.owner, &self.process.session_id)
            .await
            .map_err(|error| {
                // An error at this boundary leaves exact cleanup unproven.
                // Quarantine synchronously, before the caller can issue another
                // write or process-replacement operation.
                self.mark_quarantined();
                TerminalError::Spawn(format!("kill PTY process tree: {error}"))
            })?;
        self.record_cleanup_outcome(&outcome)
    }

    fn record_cleanup_outcome(&self, outcome: &ProcessOutcome) -> Result<(), TerminalError> {
        let result = validate_cleanup_outcome(outcome);
        if result.is_err() {
            // Do not wait for the detached exit monitor to observe the same
            // terminal outcome. The kill caller must publish the fail-closed
            // state before returning its error.
            self.mark_quarantined();
        }
        result
    }

    fn mark_quarantined(&self) {
        self.quarantined.store(true, Ordering::Release);
    }

    pub(crate) fn ensure_not_quarantined(&self) -> Result<(), TerminalError> {
        if self.quarantined.load(Ordering::Acquire) {
            Err(TerminalError::Spawn(
                "PTY process ownership is quarantined after an indeterminate cleanup".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    pub fn scrollback(&self) -> Vec<u8> {
        self.scrollback
            .lock()
            .expect("PTY scrollback lock is poisoned")
            .clone()
    }

    pub fn take_dirty_scrollback(&self) -> Option<Vec<u8>> {
        if self.dirty.swap(false, Ordering::Relaxed) {
            Some(self.scrollback())
        } else {
            None
        }
    }

    pub fn subscribe_output(&self) -> broadcast::Receiver<Vec<u8>> {
        self.out_tx.subscribe()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    #[cfg(test)]
    pub(crate) fn fail_next_kill_for_test(&self) {
        self.fail_next_kill.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn quarantine_for_test(&self) {
        self.mark_quarantined();
    }
}

fn command_with_locale_precedence(
    program: String,
    args: Vec<String>,
    env: &HashMap<String, String>,
) -> CommandSpec {
    #[cfg(unix)]
    {
        // Process requests inherit the backend environment. Preserve the old
        // terminal contract where a session-level LC_CTYPE/LANG override is not
        // silently shadowed by a stronger inherited locale category. `env`
        // execs the requested program in place, so PID/session ownership stays
        // with the runtime watchdog.
        let removals: &[&str] = if env.contains_key("LC_ALL") {
            &[]
        } else if env.contains_key("LC_CTYPE") {
            &["LC_ALL"]
        } else if env.contains_key("LANG") {
            &["LC_ALL", "LC_CTYPE"]
        } else {
            &[]
        };
        if !removals.is_empty() {
            let mut wrapped = Vec::with_capacity(removals.len() * 2 + 1 + args.len());
            for key in removals {
                wrapped.push(OsString::from("-u"));
                wrapped.push(OsString::from(key));
            }
            wrapped.push(OsString::from(program));
            wrapped.extend(args.into_iter().map(OsString::from));
            return CommandSpec::Program {
                program: OsString::from("/usr/bin/env"),
                args: wrapped,
            };
        }
    }

    #[cfg(windows)]
    if Path::new(&program)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["cmd", "bat", "ps1"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
    {
        // CreateProcessW cannot execute package-manager script shims directly.
        // Invoke them through the runtime's trusted, encoded PowerShell path.
        // Every token is a single-quoted PowerShell literal, so caller-supplied
        // arguments cannot become script syntax.
        return CommandSpec::Shell {
            shell: ShellKind::PowerShell,
            script: powershell_script_invocation(&program, &args),
        };
    }

    CommandSpec::Program {
        program: OsString::from(program),
        args: args.into_iter().map(OsString::from).collect(),
    }
}

#[cfg(any(windows, test))]
fn powershell_script_invocation(program: &str, args: &[String]) -> String {
    let mut script = String::from("& ");
    push_powershell_single_quoted(&mut script, program);
    for arg in args {
        script.push(' ');
        push_powershell_single_quoted(&mut script, arg);
    }
    script
}

#[cfg(any(windows, test))]
fn push_powershell_single_quoted(output: &mut String, value: &str) {
    output.push('\'');
    output.push_str(&value.replace('\'', "''"));
    output.push('\'');
}

fn cleanup_report(outcome: &ProcessOutcome) -> Option<&CleanupReport> {
    match outcome {
        ProcessOutcome::Exited { cleanup, .. }
        | ProcessOutcome::Cancelled { cleanup, .. }
        | ProcessOutcome::TimedOut { cleanup, .. }
        | ProcessOutcome::Lost { cleanup, .. } => Some(cleanup),
        ProcessOutcome::SpawnFailed(_) => None,
    }
}

fn validate_cleanup_outcome(outcome: &ProcessOutcome) -> Result<(), TerminalError> {
    match outcome {
        ProcessOutcome::Lost { cleanup, .. } => Err(TerminalError::Spawn(format!(
            "PTY process-tree cleanup was lost (reaped={}, errors={:?})",
            cleanup.reaped, cleanup.errors
        ))),
        ProcessOutcome::SpawnFailed(failure) => Err(TerminalError::Spawn(format!(
            "PTY process failed after activation: {} ({})",
            failure.message, failure.code
        ))),
        _ if cleanup_report(outcome).is_some_and(|cleanup| cleanup.reaped) => Ok(()),
        _ => {
            let cleanup = cleanup_report(outcome)
                .expect("every non-spawn terminal outcome carries cleanup evidence");
            Err(TerminalError::Spawn(format!(
                "PTY process-tree cleanup was not proven (errors={:?})",
                cleanup.errors
            )))
        }
    }
}

fn exit_from_outcome(outcome: &ProcessOutcome) -> PtyExit {
    match outcome {
        ProcessOutcome::Exited { code, .. } => PtyExit::Exited(*code),
        ProcessOutcome::Cancelled { .. } => PtyExit::Exited(None),
        ProcessOutcome::TimedOut { cleanup, .. } => PtyExit::Lost {
            message: format!(
                "PTY process hit an unexpected runtime deadline (errors={:?})",
                cleanup.errors
            ),
            cleanup_reaped: cleanup.reaped,
        },
        ProcessOutcome::Lost { cleanup, .. } => PtyExit::Lost {
            message: format!(
                "PTY process ownership was lost (reaped={}, errors={:?})",
                cleanup.reaped, cleanup.errors
            ),
            cleanup_reaped: cleanup.reaped,
        },
        ProcessOutcome::SpawnFailed(failure) => PtyExit::Lost {
            message: format!(
                "PTY process failed after activation: {} ({})",
                failure.message, failure.code
            ),
            cleanup_reaped: false,
        },
    }
}

fn append_scrollback(scrollback: &Arc<Mutex<Vec<u8>>>, chunk: &[u8]) {
    let mut scrollback = scrollback
        .lock()
        .expect("PTY scrollback lock is poisoned");
    scrollback.extend_from_slice(chunk);
    if scrollback.len() > SCROLLBACK_CAP {
        let overflow = scrollback.len() - SCROLLBACK_CAP;
        scrollback.drain(0..overflow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_command(script: &str) -> (String, Vec<String>) {
        #[cfg(windows)]
        {
            (
                std::env::var("ComSpec")
                    .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_owned()),
                vec!["/d".to_owned(), "/c".to_owned(), script.to_owned()],
            )
        }
        #[cfg(not(windows))]
        {
            (
                "/bin/sh".to_owned(),
                vec!["-c".to_owned(), script.to_owned()],
            )
        }
    }

    #[test]
    fn scrollback_is_bounded_and_keeps_recent_bytes() {
        let scrollback = Arc::new(Mutex::new(Vec::<u8>::new()));
        append_scrollback(&scrollback, &vec![b'a'; SCROLLBACK_CAP]);
        append_scrollback(&scrollback, b"TAIL");
        let data = scrollback.lock().unwrap();
        assert_eq!(data.len(), SCROLLBACK_CAP);
        assert_eq!(&data[data.len() - 4..], b"TAIL");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_character_locale_removes_stronger_inherited_categories() {
        let command = command_with_locale_precedence(
            "sh".to_owned(),
            vec!["-l".to_owned()],
            &HashMap::from([("LANG".to_owned(), "en_US.UTF-8".to_owned())]),
        );
        let CommandSpec::Program { program, args } = command else {
            panic!("locale wrapper must remain a direct program request");
        };
        assert_eq!(program, OsString::from("/usr/bin/env"));
        assert_eq!(
            args,
            [
                "-u",
                "LC_ALL",
                "-u",
                "LC_CTYPE",
                "sh",
                "-l",
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn powershell_script_shim_invocation_quotes_every_token_as_data() {
        assert_eq!(
            powershell_script_invocation(
                r"C:\Agent's Tools\runner.cmd",
                &["a'b".to_owned(), String::new(), "$(Get-Item env:PATH)".to_owned()]
            ),
            r"& 'C:\Agent''s Tools\runner.cmd' 'a''b' '' '$(Get-Item env:PATH)'"
        );
    }

    #[test]
    fn lost_outcome_is_never_accepted_as_success_even_when_reaped() {
        let now = Instant::now();
        let outcome = ProcessOutcome::Lost {
            last_known: nomi_process_runtime::ProcessSnapshot {
                pid: 42,
                state: nomi_process_runtime::ProcessState::Lost,
                started_at: now,
                last_activity_at: now,
            },
            output: Default::default(),
            cleanup: CleanupReport {
                reaped: true,
                ..CleanupReport::default()
            },
        };

        assert!(
            validate_cleanup_outcome(&outcome).is_err(),
            "Lost ownership must remain an error even after exact cleanup"
        );
        assert!(matches!(
            exit_from_outcome(&outcome),
            PtyExit::Lost {
                cleanup_reaped: true,
                ..
            }
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_cleanup_outcome_quarantines_before_return() {
        #[cfg(windows)]
        let script = "ping -n 60 127.0.0.1 >NUL";
        #[cfg(not(windows))]
        let script = "sleep 60";
        let (program, args) = shell_command(script);
        let handle = PtyHandle::spawn(
            SpawnParams {
                program,
                args,
                cwd: String::new(),
                env: HashMap::new(),
                cols: 80,
                rows: 24,
            },
            0,
            |_chunk| {},
            |_exit, _scrollback| {},
        )
        .await
        .expect("spawn");

        let now = Instant::now();
        let lost_but_reaped = ProcessOutcome::Lost {
            last_known: nomi_process_runtime::ProcessSnapshot {
                pid: handle.pid().expect("runtime leader pid"),
                state: nomi_process_runtime::ProcessState::Lost,
                started_at: now,
                last_activity_at: now,
            },
            output: Default::default(),
            cleanup: CleanupReport {
                reaped: true,
                ..CleanupReport::default()
            },
        };
        assert!(handle.record_cleanup_outcome(&lost_but_reaped).is_err());
        assert!(
            handle.ensure_not_quarantined().is_err(),
            "Lost must publish quarantine before the kill caller receives its error"
        );

        // Exercise the independent `cleanup.reaped=false` condition on the
        // same live runtime authority.
        handle.quarantined.store(false, Ordering::Release);
        let unreaped = ProcessOutcome::Cancelled {
            output: Default::default(),
            cleanup: CleanupReport::default(),
        };
        assert!(handle.record_cleanup_outcome(&unreaped).is_err());
        assert!(
            handle.ensure_not_quarantined().is_err(),
            "unproven reap must publish quarantine synchronously"
        );

        // Explicit cleanup remains possible; quarantine blocks writes and
        // replacement/deletion, not the only operation that can prove safety.
        handle.kill().await.expect("clean up real test process");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quick_exit_is_observed_after_delayed_activation() {
        #[cfg(windows)]
        let script = "<nul set /p \"=quick\" & exit /b 0";
        #[cfg(not(windows))]
        let script = "printf quick";
        let (program, args) = shell_command(script);
        let exited = Arc::new(tokio::sync::Notify::new());
        let exited_callback = Arc::clone(&exited);
        let handle = PtyHandle::spawn(
            SpawnParams {
                program,
                args,
                cwd: String::new(),
                env: HashMap::new(),
                cols: 80,
                rows: 24,
            },
            7,
            |_chunk| {},
            move |exit, _scrollback| {
                assert!(matches!(exit, PtyExit::Exited(Some(0))));
                exited_callback.notify_one();
            },
        )
        .await
        .expect("spawn quick process");

        // Let the child finish before the service-equivalent registration gate.
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.activate();
        tokio::time::timeout(Duration::from_secs(3), exited.notified())
            .await
            .expect("delayed activation must still observe the terminal outcome");
        assert!(
            String::from_utf8_lossy(&handle.scrollback()).contains("quick"),
            "output emitted before activation must remain available"
        );
        assert_eq!(handle.epoch(), 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dirty_scrollback_and_live_output_preserve_raw_bytes() {
        #[cfg(windows)]
        let script = "<nul set /p \"=hello\" & exit /b 0";
        #[cfg(not(windows))]
        let script = "printf hello";
        let (program, args) = shell_command(script);
        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let captured_callback = Arc::clone(&captured);
        let exited = Arc::new(tokio::sync::Notify::new());
        let exited_callback = Arc::clone(&exited);
        let handle = PtyHandle::spawn(
            SpawnParams {
                program,
                args,
                cwd: String::new(),
                env: HashMap::new(),
                cols: 80,
                rows: 24,
            },
            0,
            move |chunk| captured_callback.lock().unwrap().extend_from_slice(&chunk),
            move |_exit, _scrollback| exited_callback.notify_one(),
        )
        .await
        .expect("spawn");
        handle.activate();

        tokio::time::timeout(Duration::from_secs(3), exited.notified())
            .await
            .expect("process should exit");
        let snapshot = handle
            .take_dirty_scrollback()
            .expect("output must set dirty");
        assert!(String::from_utf8_lossy(&snapshot).contains("hello"));
        assert!(String::from_utf8_lossy(&captured.lock().unwrap()).contains("hello"));
        assert!(handle.take_dirty_scrollback().is_none());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn force_kill_reaps_the_owned_process_group() {
        let handle = PtyHandle::spawn(
            SpawnParams {
                program: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), "sleep 60 & wait".to_owned()],
                cwd: String::new(),
                env: HashMap::new(),
                cols: 80,
                rows: 24,
            },
            0,
            |_chunk| {},
            |_exit, _scrollback| {},
        )
        .await
        .expect("spawn");
        handle.activate();
        let pid = handle.pid().expect("runtime must expose leader pid") as i32;
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);

        handle.kill().await.expect("exact process-group cleanup");
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("PTY leader remained alive after exact force-kill");
    }
}
