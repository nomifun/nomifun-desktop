//! Engine-neutral owner adapter for `nomi-process-runtime`.
//!
//! This module does not expose a model Tool and is not called directly by the
//! engine turn loop. The application-owned Wave2/Kernel host adapter may use
//! it to implement `workspace.process/exec` and interactive process-session actions
//! without duplicating process ownership.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nomi_process_runtime::{
    CapabilityPolicy, CleanupReport, CommandSpec, EncodingMetadata, NormalizedProcessRequest,
    MAX_PTY_DIMENSION, OutputCursor, OutputSnapshot, PollResult, ProcessError, ProcessOutcome,
    ProcessOwner, ProcessPolicy, ProcessRequest, ProcessSupervisor, SandboxPolicy, SessionId,
    SupervisorConfig, Transport, ShellKind, normalize_request,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::EngineProcessError;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_LIMIT_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMMAND_CHARS: usize = 32 * 1024;
const MAX_ARGUMENTS: usize = 256;
const MAX_ARGUMENT_CHARS: usize = 64 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 128;
const MAX_POLL_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineProcessTransport {
    Pipe,
    Pty { cols: u16, rows: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineProcessRequest {
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_script: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_output_limit_bytes")]
    pub output_limit_bytes: usize,
    pub transport: EngineProcessTransport,
}

impl EngineProcessRequest {
    pub fn pipe(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            shell_script: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            output_limit_bytes: DEFAULT_OUTPUT_LIMIT_BYTES,
            transport: EngineProcessTransport::Pipe,
        }
    }

    pub fn shell(script: impl Into<String>) -> Self {
        Self { shell_script: Some(script.into()), ..Self::pipe("") }
    }

    pub fn validate(&self) -> Result<(), EngineProcessError> {
        if let Some(script) = &self.shell_script {
            if !self.command.is_empty() || !self.args.is_empty() || script.trim().is_empty()
                || script.contains('\0') || script.chars().count() > MAX_COMMAND_CHARS {
                return Err(EngineProcessError::Process("shell script must be bounded and cannot include program/argv fields".into()));
            }
        } else if self.command.trim().is_empty()
            || self.command.trim() != self.command
            || self.command.contains('\0')
            || self.command.chars().count() > MAX_COMMAND_CHARS
        {
            return Err(EngineProcessError::Process(
                "command must be a bounded executable without edge whitespace or NUL bytes"
                    .to_owned(),
            ));
        }
        if self.args.len() > MAX_ARGUMENTS
            || self.args.iter().any(|argument| {
                argument.contains('\0') || argument.chars().count() > MAX_ARGUMENT_CHARS
            })
        {
            return Err(EngineProcessError::Process(
                "process arguments exceed the engine process limits".to_owned(),
            ));
        }
        if self.env.len() > MAX_ENVIRONMENT_ENTRIES
            || self.env.iter().any(|(key, value)| {
                key.is_empty()
                    || key.contains(['=', '\0'])
                    || value.contains('\0')
                    || key.len() > 1024
                    || value.len() > MAX_ARGUMENT_CHARS
            })
        {
            return Err(EngineProcessError::Process(
                "process environment contains an invalid or oversized entry".to_owned(),
            ));
        }
        if !(1..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(EngineProcessError::Process(format!(
                "timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"
            )));
        }
        if self.output_limit_bytes == 0 || self.output_limit_bytes > MAX_OUTPUT_LIMIT_BYTES {
            return Err(EngineProcessError::Process(format!(
                "output_limit_bytes must be between 1 and {MAX_OUTPUT_LIMIT_BYTES}"
            )));
        }
        if let EngineProcessTransport::Pty { cols, rows } = self.transport
            && (!(1..=MAX_PTY_DIMENSION).contains(&cols)
                || !(1..=MAX_PTY_DIMENSION).contains(&rows))
        {
            return Err(EngineProcessError::Process(format!(
                "PTY dimensions must be between 1 and {MAX_PTY_DIMENSION}"
            )));
        }
        if self.cwd.as_deref().is_some_and(invalid_relative_path) {
            return Err(EngineProcessError::Process(
                "cwd must be a normalized workspace-relative path".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct EngineProcessSession {
    owner: ProcessOwner,
    session_id: SessionId,
    pid: u32,
    cursor: OutputCursor,
}

impl EngineProcessSession {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn session_id(&self) -> String {
        self.session_id.to_string()
    }

    pub fn cursor(&self) -> u64 {
        self.cursor.offset()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineProcessOutput {
    pub text: String,
    pub next_cursor: u64,
    pub retained_bytes: usize,
    pub dropped_bytes: u64,
    pub source_encoding: String,
    pub decode_errors: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineProcessPoll {
    Running {
        pid: u32,
        output: EngineProcessOutput,
    },
    Exited {
        exit_code: Option<i32>,
        signal: Option<i32>,
        output: EngineProcessOutput,
        cleanup: EngineCleanupReport,
    },
    Cancelled {
        output: EngineProcessOutput,
        cleanup: EngineCleanupReport,
    },
    TimedOut {
        output: EngineProcessOutput,
        cleanup: EngineCleanupReport,
    },
    Lost {
        pid: u32,
        output: EngineProcessOutput,
        cleanup: EngineCleanupReport,
    },
    SpawnFailed {
        code: String,
        message: String,
    },
}

impl EngineProcessPoll {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineCleanupReport {
    pub interrupt_attempted: bool,
    pub terminate_attempted: bool,
    pub force_kill_attempted: bool,
    pub reaped: bool,
    pub elapsed_ms: u64,
    pub errors: Vec<String>,
}

pub struct ManagedEngineProcessOwner {
    workspace_root: PathBuf,
    supervisor: Arc<ProcessSupervisor>,
    pending_starts: Mutex<HashMap<ProcessOwner, bool>>,
}

/// Start failed, with the owner's structured cleanup fact preserved before
/// converting the cause to an engine error. Not command success or rollback.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct EngineProcessStartError {
    #[source]
    pub error: EngineProcessError,
    pub no_live_process_proven: bool,
    /// Stronger than cleanup: the requested program never began execution.
    pub user_code_not_started: bool,
}

impl ManagedEngineProcessOwner {
    pub fn new(
        workspace_root: impl AsRef<Path>,
        config: SupervisorConfig,
    ) -> Result<Self, EngineProcessError> {
        let workspace_root = std::fs::canonicalize(workspace_root.as_ref()).map_err(|error| {
            EngineProcessError::Process(format!(
                "workspace root {} is unavailable: {error}",
                workspace_root.as_ref().display()
            ))
        })?;
        if !workspace_root.is_dir() {
            return Err(EngineProcessError::Process(
                "workspace root is not a directory".to_owned(),
            ));
        }
        Ok(Self {
            workspace_root,
            supervisor: ProcessSupervisor::new(config),
            pending_starts: Mutex::new(HashMap::new()),
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub async fn start(
        &self,
        request: EngineProcessRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineProcessSession, EngineProcessError> {
        self.start_with_evidence(request, cancellation).await.map_err(|failure| failure.error)
    }

    pub async fn start_with_evidence(
        &self,
        request: EngineProcessRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineProcessSession, EngineProcessStartError> {
        let before_spawn = |error| EngineProcessStartError { error, no_live_process_proven: true, user_code_not_started: true };
        request.validate().map_err(before_spawn)?;
        if cancellation.is_cancelled() {
            return Err(before_spawn(EngineProcessError::Cancelled));
        }
        let normalized = self.normalized_request(request).map_err(before_spawn)?;
        let native_owner = normalized.owner.clone();
        // Stamp before the first native await. Dropping this adapter future
        // cannot erase the identity needed to reconcile its retained worker.
        self.pending_starts.lock().expect("process startup tracking is poisoned")
            .insert(native_owner.clone(), false);
        // Complete the ownership transaction even when cancellation races
        // spawn; callers must never lose a newly committed process handle.
        let handle = self
            .supervisor
            .start(normalized)
            .await
            .map_err(|cause| {
                let no_live_process_proven = match &cause {
                    ProcessError::InvalidWorkingDirectory { .. }
                    | ProcessError::CapabilityDenied { .. }
                    | ProcessError::InvalidCommand { .. }
                    | ProcessError::InvalidTransport { .. }
                    | ProcessError::CapacityExhausted { .. }
                    | ProcessError::SupervisorShuttingDown
                    | ProcessError::SpawnFailed { .. } => true,
                    ProcessError::StartLost { cleanup, .. } => cleanup.reaped,
                    _ => false,
                };
                let user_code_not_started = matches!(&cause,
                    ProcessError::InvalidWorkingDirectory { .. }
                    | ProcessError::CapabilityDenied { .. }
                    | ProcessError::InvalidCommand { .. }
                    | ProcessError::InvalidTransport { .. }
                    | ProcessError::CapacityExhausted { .. }
                    | ProcessError::SupervisorShuttingDown
                    | ProcessError::SpawnFailed { .. });
                if no_live_process_proven {
                    self.pending_starts.lock().expect("process startup tracking is poisoned").remove(&native_owner);
                }
                EngineProcessStartError { error: process_error(cause), no_live_process_proven, user_code_not_started }
            })?;
        self.pending_starts.lock().expect("process startup tracking is poisoned").remove(&native_owner);
        let session = EngineProcessSession {
            owner: handle.owner,
            session_id: handle.session_id,
            pid: handle.pid,
            cursor: OutputCursor::START,
        };
        // Even on cancellation return the handle. wait/poll observes the
        // token and the caller retains ownership if cleanup itself fails.
        Ok(session)
    }

    /// Reconcile starts that never produced an adapter handle using their
    /// exact host-generated owners, not an empty handle map or another call.
    pub async fn quiesce_unregistered_starts(&self) -> bool {
        if self.pending_starts.lock().expect("process startup tracking is poisoned").is_empty() { return false; }
        let report = self.supervisor.quiesce().await;
        let mut tracked = self.pending_starts.lock().expect("process startup tracking is poisoned");
        settle_pending_starts_from_fence(&mut tracked, &report)
    }

    pub async fn execute(
        &self,
        request: EngineProcessRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineProcessPoll, EngineProcessError> {
        let mut session = self.start(request, cancellation.clone()).await?;
        self.wait(&mut session, cancellation).await
    }

    pub async fn poll(
        &self,
        session: &mut EngineProcessSession,
        wait: Duration,
        cancellation: CancellationToken,
    ) -> Result<EngineProcessPoll, EngineProcessError> {
        self.poll_from(session, session.cursor(), wait, cancellation)
            .await
    }

    pub async fn poll_from(
        &self,
        session: &mut EngineProcessSession,
        cursor: u64,
        wait: Duration,
        cancellation: CancellationToken,
    ) -> Result<EngineProcessPoll, EngineProcessError> {
        if cancellation.is_cancelled() {
            return self.cancel(session).await;
        }
        let wait = wait.min(MAX_POLL_WAIT);
        let poll = self.supervisor.poll_until_activity(
            &session.owner,
            &session.session_id,
            OutputCursor::new(cursor),
            Instant::now() + wait,
        );
        let result = tokio::select! {
            _ = cancellation.cancelled() => return self.cancel(session).await,
            result = poll => result.map_err(process_error)?,
        };
        Ok(self.convert_poll(session, result))
    }

    pub async fn wait(
        &self,
        session: &mut EngineProcessSession,
        cancellation: CancellationToken,
    ) -> Result<EngineProcessPoll, EngineProcessError> {
        loop {
            if cancellation.is_cancelled() {
                return self.cancel(session).await;
            }
            let poll = self.supervisor.poll(
                &session.owner,
                &session.session_id,
                OutputCursor::START,
                Instant::now() + MAX_POLL_WAIT,
            );
            let result = tokio::select! {
                _ = cancellation.cancelled() => return self.cancel(session).await,
                result = poll => result.map_err(process_error)?,
            };
            match result {
                PollResult::Running { .. } => continue,
                PollResult::Finished(outcome) => {
                    return Ok(self.convert_outcome(session, outcome));
                }
            }
        }
    }

    pub async fn write_stdin(
        &self,
        session: &EngineProcessSession,
        bytes: &[u8],
        cancellation: CancellationToken,
    ) -> Result<(), EngineProcessError> {
        if bytes.len() > 1024 * 1024 {
            return Err(EngineProcessError::Process(
                "one stdin write may not exceed 1 MiB".to_owned(),
            ));
        }
        let write = self
            .supervisor
            .write(&session.owner, &session.session_id, bytes);
        tokio::select! {
            _ = cancellation.cancelled() => Err(EngineProcessError::Cancelled),
            result = write => result.map_err(process_error),
        }
    }

    pub async fn close_stdin(
        &self,
        session: &EngineProcessSession,
    ) -> Result<(), EngineProcessError> {
        self.supervisor
            .close_stdin(&session.owner, &session.session_id)
            .await
            .map_err(process_error)
    }

    pub async fn resize(
        &self,
        session: &EngineProcessSession,
        cols: u16,
        rows: u16,
    ) -> Result<(), EngineProcessError> {
        self.supervisor
            .resize(&session.owner, &session.session_id, cols, rows)
            .await
            .map_err(process_error)
    }

    pub async fn cancel(
        &self,
        session: &mut EngineProcessSession,
    ) -> Result<EngineProcessPoll, EngineProcessError> {
        let outcome = self
            .supervisor
            .cancel(&session.owner, &session.session_id)
            .await
            .map_err(process_error)?;
        Ok(self.convert_outcome(session, outcome))
    }

    fn normalized_request(
        &self,
        request: EngineProcessRequest,
    ) -> Result<NormalizedProcessRequest, EngineProcessError> {
        let timeout = Duration::from_millis(request.timeout_ms);
        let transport = match request.transport {
            EngineProcessTransport::Pipe => Transport::Pipe,
            EngineProcessTransport::Pty { cols, rows } => Transport::Pty { cols, rows },
        };
        let mut policy = ProcessPolicy::default();
        policy.output_limit_bytes = request.output_limit_bytes;
        policy.lease = timeout.saturating_add(Duration::from_secs(60));
        policy.deadline = Some(Instant::now() + timeout);
        let request = ProcessRequest {
            owner: ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7()),
            command: match request.shell_script {
                Some(script) => CommandSpec::Shell {
                    shell: if cfg!(windows) { ShellKind::PowerShell } else { ShellKind::Posix }, script,
                },
                None => CommandSpec::Program {
                    program: OsString::from(request.command),
                    args: request.args.into_iter().map(OsString::from).collect(),
                },
            },
            cwd: request
                .cwd
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
            env: request
                .env
                .into_iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value)))
                .collect(),
            transport,
            policy,
            capability: CapabilityPolicy {
                cwd_roots: vec![self.workspace_root.clone()],
                sandbox: workspace_process_sandbox(&self.workspace_root),
            },
        };
        normalize_request(request, &self.workspace_root).map_err(process_error)
    }

    fn convert_poll(
        &self,
        session: &mut EngineProcessSession,
        result: PollResult,
    ) -> EngineProcessPoll {
        match result {
            PollResult::Running { snapshot, output } => {
                debug_assert_eq!(snapshot.pid, session.pid);
                session.cursor = output.next_cursor;
                EngineProcessPoll::Running {
                    pid: snapshot.pid,
                    output: convert_output(output),
                }
            }
            PollResult::Finished(outcome) => self.convert_outcome(session, outcome),
        }
    }

    fn convert_outcome(
        &self,
        session: &mut EngineProcessSession,
        outcome: ProcessOutcome,
    ) -> EngineProcessPoll {
        match outcome {
            ProcessOutcome::Exited {
                code,
                signal,
                output,
                cleanup,
            } => {
                session.cursor = output.next_cursor;
                EngineProcessPoll::Exited {
                    exit_code: code,
                    signal,
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::Cancelled { output, cleanup } => {
                session.cursor = output.next_cursor;
                EngineProcessPoll::Cancelled {
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::TimedOut { output, cleanup } => {
                session.cursor = output.next_cursor;
                EngineProcessPoll::TimedOut {
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::Lost {
                last_known,
                output,
                cleanup,
            } => {
                session.cursor = output.next_cursor;
                EngineProcessPoll::Lost {
                    pid: last_known.pid,
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::SpawnFailed(failure) => EngineProcessPoll::SpawnFailed {
                code: failure.code,
                message: failure.message,
            },
        }
    }
}

fn settle_pending_starts_from_fence(pending: &mut HashMap<ProcessOwner, bool>, report: &nomi_process_runtime::QuiesceReport) -> bool {
    if pending.is_empty() { return false; }
    for (owner, proven) in pending.iter_mut() {
        *proven |= report.startups.iter().any(|entry| &entry.owner == owner && entry.cleanup.reaped)
            || report.sessions.iter().any(|entry| &entry.owner == owner && match &entry.outcome {
                ProcessOutcome::Exited { cleanup, .. } | ProcessOutcome::Cancelled { cleanup, .. }
                | ProcessOutcome::TimedOut { cleanup, .. } | ProcessOutcome::Lost { cleanup, .. } => cleanup.reaped,
                ProcessOutcome::SpawnFailed(_) => false,
            });
    }
    // Preserve partial exact receipts across cleanup retries. Quiesce consumes
    // completed native startup reports, so forgetting one would strand its ID.
    if !report.is_exact() || !pending.values().all(|proven| *proven) { return false; }
    pending.clear();
    true
}

fn invalid_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    value.trim().is_empty()
        || value.trim() != value
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains('\0')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
}

fn workspace_process_sandbox(workspace_root: &Path) -> SandboxPolicy {
    #[cfg(target_os = "macos")]
    {
        return SandboxPolicy::MacSeatbelt {
            write_roots: vec![workspace_root.to_path_buf()],
        };
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = workspace_root;
        SandboxPolicy::UnrestrictedLocalOwner
    }
}

fn convert_output(output: OutputSnapshot) -> EngineProcessOutput {
    let text = output.text();
    let EncodingMetadata {
        source_encoding,
        decode_errors,
    } = output.encoding;
    EngineProcessOutput {
        text,
        next_cursor: output.next_cursor.offset(),
        retained_bytes: output.retained_bytes,
        dropped_bytes: output.dropped_bytes,
        source_encoding,
        decode_errors,
    }
}

fn convert_cleanup(cleanup: CleanupReport) -> EngineCleanupReport {
    EngineCleanupReport {
        interrupt_attempted: cleanup.interrupt_attempted,
        terminate_attempted: cleanup.terminate_attempted,
        force_kill_attempted: cleanup.force_kill_attempted,
        reaped: cleanup.reaped,
        elapsed_ms: cleanup.elapsed.as_millis().min(u64::MAX as u128) as u64,
        errors: cleanup.errors,
    }
}

fn process_error(error: ProcessError) -> EngineProcessError {
    EngineProcessError::Process(format!("{}: {error}", error.code()))
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

const fn default_output_limit_bytes() -> usize {
    DEFAULT_OUTPUT_LIMIT_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn startup_fence_report(owner: ProcessOwner, reaped: bool) -> nomi_process_runtime::QuiesceReport {
        nomi_process_runtime::QuiesceReport {
            sessions: Vec::new(), errors: Vec::new(),
            startups: vec![nomi_process_runtime::StartupCleanupReport {
                session_id: SessionId::new(), owner,
                failure: nomi_process_runtime::SpawnFailure { code: "spawn_cleanup_deferred".into(), message: "controlled receipt".into() },
                cleanup: CleanupReport { reaped, ..CleanupReport::default() }, user_code_not_started: true,
            }],
        }
    }

    #[test]
    fn startup_fence_requires_exact_native_owner_not_an_empty_or_foreign_report() {
        let owner = ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7());
        let mut pending = HashMap::from([(owner.clone(), false)]);
        let empty = nomi_process_runtime::QuiesceReport { sessions: Vec::new(), startups: Vec::new(), errors: Vec::new() };
        assert!(!settle_pending_starts_from_fence(&mut pending, &empty));
        let foreign = ProcessOwner::new(owner.invocation_id, Uuid::now_v7());
        assert!(!settle_pending_starts_from_fence(&mut pending, &startup_fence_report(foreign, true)));
        assert!(!settle_pending_starts_from_fence(&mut pending, &startup_fence_report(owner.clone(), false)));
        assert_eq!(pending.get(&owner), Some(&false));
        assert!(settle_pending_starts_from_fence(&mut pending, &startup_fence_report(owner, true)));
        assert!(pending.is_empty());
    }

    #[test]
    fn startup_fence_retains_partial_exact_receipts_until_all_starts_are_proven() {
        let first = ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7());
        let second = ProcessOwner::new(first.invocation_id, Uuid::now_v7());
        let mut pending = HashMap::from([(first.clone(), false), (second.clone(), false)]);
        assert!(!settle_pending_starts_from_fence(&mut pending, &startup_fence_report(first.clone(), true)));
        assert_eq!(pending.get(&first), Some(&true));
        assert_eq!(pending.get(&second), Some(&false));
        assert!(settle_pending_starts_from_fence(&mut pending, &startup_fence_report(second, true)));
        assert!(pending.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_pipe_unregistered_start_is_reconciled_by_its_exact_owner_fence() {
        native_unregistered_start_fence(EngineProcessTransport::Pipe).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_pty_unregistered_start_is_reconciled_by_its_exact_owner_fence() {
        native_unregistered_start_fence(EngineProcessTransport::Pty { cols: 80, rows: 24 }).await;
    }

    #[cfg(unix)]
    async fn native_unregistered_start_fence(transport: EngineProcessTransport) {
        let temporary = tempfile::tempdir().unwrap();
        let scenario = if matches!(transport, EngineProcessTransport::Pipe) { "engine-pipe-drop" } else { "engine-pty-drop" };
        let root = std::env::var_os("NOMI_ENGINE_FENCE_EVIDENCE").map(PathBuf::from)
            .map(|root| root.join(scenario)).unwrap_or_else(|| temporary.path().to_path_buf());
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let marker = root.join("executed.pid");
        let owner = ManagedEngineProcessOwner::new(&root, SupervisorConfig::default()).unwrap();
        let mut request = EngineProcessRequest::pipe("/bin/sh");
        request.args = vec!["-c".into(), "printf '%s\\n' \"$$\" > \"$1\"; exec /bin/sleep 60".into(),
            "engine-fence-fixture".into(), marker.to_string_lossy().to_string()];
        request.transport = transport;
        let mut start = Box::pin(owner.start_with_evidence(request, CancellationToken::new()));
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(start.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        }).await;
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(contents) = std::fs::read_to_string(&marker) {
                    if contents.ends_with('\n') { break contents.trim().parse::<u32>().unwrap(); }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        let before = owner.pending_starts.lock().unwrap().clone();
        drop(start);
        let exact = tokio::time::timeout(Duration::from_secs(6), owner.quiesce_unregistered_starts()).await.unwrap();
        let gone = nomi_process_runtime::probe_process_identity(pid).unwrap().is_none();
        let tracked_after = owner.pending_starts.lock().unwrap().len();
        std::fs::write(root.join("assertions.json"), serde_json::json!({
            "pid":pid,"native_tracking_count":before.len(),
            "native_owners":before.keys().map(|owner| format!("{}:{}",owner.invocation_id,owner.call_id)).collect::<Vec<_>>(),
            "caller_future_dropped":true,"exact_owner_fence":exact,"physical_pid_gone":gone,
            "tracked_after":tracked_after,
        }).to_string()).unwrap();
        assert!(before.len() == 1 && exact && gone && tracked_after == 0);
    }

    fn echo_request() -> EngineProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
            vec![
                "/d".to_owned(),
                "/c".to_owned(),
                "echo coding-engine".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "printf coding-engine".to_owned()],
        );
        EngineProcessRequest {
            command,
            shell_script: None,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 10_000,
            output_limit_bytes: 64 * 1024,
            transport: EngineProcessTransport::Pipe,
        }
    }

    fn stdin_request() -> EngineProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
            vec![
                "/d".to_owned(),
                "/v:on".to_owned(),
                "/c".to_owned(),
                "set /p line=& echo !line!".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                "IFS= read -r line; printf %s \"$line\"".to_owned(),
            ],
        );
        EngineProcessRequest {
            command,
            shell_script: None,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 10_000,
            output_limit_bytes: 64 * 1024,
            transport: EngineProcessTransport::Pipe,
        }
    }

    fn stdin_count_request() -> EngineProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            "powershell.exe".to_owned(),
            vec![
                "-NoProfile".to_owned(),
                "-Command".to_owned(),
                "$text=[Console]::In.ReadToEnd(); [Console]::Out.Write([Text.Encoding]::UTF8.GetByteCount($text))".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "wc -c".to_owned()],
        );
        EngineProcessRequest {
            command,
            shell_script: None,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 10_000,
            output_limit_bytes: 64 * 1024,
            transport: EngineProcessTransport::Pipe,
        }
    }

    #[test]
    fn pty_dimensions_above_the_portable_signed_limit_are_rejected() {
        let mut request = echo_request();
        request.transport = EngineProcessTransport::Pty {
            cols: 32768,
            rows: 40,
        };
        let error = request
            .validate()
            .expect_err("dimensions above the Windows COORD limit must fail before spawn");
        assert!(error.to_string().contains("between 1 and 32767"));
    }

    fn sleeper_request() -> EngineProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
            vec![
                "/d".to_owned(),
                "/c".to_owned(),
                "ping -n 31 127.0.0.1 >nul".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "sleep 30".to_owned()],
        );
        EngineProcessRequest {
            command,
            shell_script: None,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 60_000,
            output_limit_bytes: 64 * 1024,
            transport: EngineProcessTransport::Pipe,
        }
    }

    fn long_output_request() -> EngineProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
            vec![
                "/d".to_owned(),
                "/c".to_owned(),
                "for /L %i in (1,1,200) do @echo 1234567890".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                "i=0; while [ $i -lt 200 ]; do echo 1234567890; i=$((i+1)); done".to_owned(),
            ],
        );
        EngineProcessRequest {
            command,
            shell_script: None,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 10_000,
            output_limit_bytes: 256,
            transport: EngineProcessTransport::Pipe,
        }
    }

    #[tokio::test]
    async fn managed_owner_runs_and_reaps_a_bounded_process() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(echo_request(), CancellationToken::new())
            .await
            .unwrap();
        let outcome = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            exit_code,
            output,
            cleanup,
            ..
        } = outcome
        else {
            panic!("echo command should exit normally");
        };
        assert_eq!(exit_code, Some(0));
        assert!(output.text.contains("coding-engine"));
        assert!(cleanup.reaped);
    }

    #[tokio::test]
    #[ignore = "requires Bun on PATH for a live coding process check"]
    async fn managed_owner_launches_bun_from_path() {
        let directory = tempfile::tempdir().unwrap();
        let owner = ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut request = EngineProcessRequest::pipe("bun");
        request.args = vec!["--version".into()];
        request.timeout_ms = 10_000;
        let outcome = owner.execute(request, CancellationToken::new()).await.unwrap();
        assert!(matches!(outcome, EngineProcessPoll::Exited { exit_code: Some(0), .. }));
    }

    #[tokio::test]
    async fn workspace_escape_is_rejected_before_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut request = echo_request();
        request.cwd = Some("../outside".to_owned());
        assert!(matches!(
            owner.start(request, CancellationToken::new()).await,
            Err(EngineProcessError::Process(_))
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_owner_seatbelt_denies_writes_outside_the_workspace() {
        // The trusted Darwin temporary directories are writable by design, so
        // place both siblings beside the checkout to exercise the declared
        // workspace write boundary rather than the trusted-temp exception.
        let fixture_root = std::env::current_dir().unwrap();
        let workspace = tempfile::tempdir_in(&fixture_root).unwrap();
        let outside = tempfile::tempdir_in(&fixture_root).unwrap();
        let inside_marker = workspace.path().join("inside.marker");
        let outside_marker = outside.path().join("outside.marker");
        let owner =
            ManagedEngineProcessOwner::new(workspace.path(), SupervisorConfig::default()).unwrap();

        for (marker, expected_exit) in [(&inside_marker, Some(0)), (&outside_marker, None)] {
            let mut request = EngineProcessRequest::pipe("/usr/bin/touch");
            request.args = vec![marker.to_string_lossy().into_owned()];
            let outcome = owner
                .execute(request, CancellationToken::new())
                .await
                .unwrap();
            let EngineProcessPoll::Exited {
                exit_code, cleanup, ..
            } = outcome
            else {
                panic!("touch should settle as an exited process");
            };
            assert!(cleanup.reaped);
            if let Some(expected_exit) = expected_exit {
                assert_eq!(exit_code, Some(expected_exit));
            } else {
                assert_ne!(exit_code, Some(0));
            }
        }

        assert!(inside_marker.exists());
        assert!(
            !outside_marker.exists(),
            "the product process owner must apply the macOS workspace write sandbox"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_owner_preserves_literal_macos_argv_without_shell_expansion() {
        let fixture_root = std::env::current_dir().unwrap();
        let workspace = tempfile::tempdir_in(&fixture_root).unwrap();
        let expansion_marker = workspace.path().join("must-not-expand.marker");
        let owner =
            ManagedEngineProcessOwner::new(workspace.path(), SupervisorConfig::default()).unwrap();
        let tokens = [
            "*".to_owned(),
            "$HOME".to_owned(),
            "`uname`".to_owned(),
            format!("$(touch {})", expansion_marker.display()),
            "semi;colon".to_owned(),
            "line\nbreak".to_owned(),
        ];
        let mut request = EngineProcessRequest::pipe("/usr/bin/printf");
        request.args = std::iter::once("<%s>\\n".to_owned())
            .chain(tokens.iter().cloned())
            .collect();

        let outcome = owner
            .execute(request, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            exit_code,
            output,
            cleanup,
            ..
        } = outcome
        else {
            panic!("literal argv probe should exit normally");
        };
        assert_eq!(exit_code, Some(0));
        assert!(cleanup.reaped);
        assert_eq!(
            output.text,
            tokens
                .iter()
                .map(|token| format!("<{token}>\n"))
                .collect::<String>()
        );
        assert!(!expansion_marker.exists());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_owner_seatbelt_rejects_tmpdir_override_before_user_code() {
        let fixture_root = std::env::current_dir().unwrap();
        let workspace = tempfile::tempdir_in(&fixture_root).unwrap();
        let marker = workspace.path().join("must-not-run.marker");
        let owner =
            ManagedEngineProcessOwner::new(workspace.path(), SupervisorConfig::default()).unwrap();
        let mut request = EngineProcessRequest::pipe("/usr/bin/touch");
        request.args = vec![marker.to_string_lossy().into_owned()];
        request
            .env
            .insert("TMPDIR".to_owned(), "/tmp/untrusted-override".to_owned());

        let failure = owner
            .start_with_evidence(request, CancellationToken::new())
            .await
            .expect_err("a model-supplied TMPDIR must not bypass the Seatbelt profile");

        assert!(failure.no_live_process_proven);
        assert!(failure.user_code_not_started);
        assert!(!marker.exists());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_owner_rejects_a_non_executable_without_shell_fallback() {
        use std::os::unix::fs::PermissionsExt;

        let fixture_root = std::env::current_dir().unwrap();
        let workspace = tempfile::tempdir_in(&fixture_root).unwrap();
        let script = workspace.path().join("not executable.sh");
        let marker = workspace.path().join("must-not-run.marker");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
        let owner =
            ManagedEngineProcessOwner::new(workspace.path(), SupervisorConfig::default()).unwrap();
        let request = EngineProcessRequest::pipe(script.to_string_lossy());

        let failure = owner
            .start_with_evidence(request, CancellationToken::new())
            .await
            .expect_err("a non-executable file must fail before user code starts");

        assert!(failure.no_live_process_proven);
        assert!(failure.user_code_not_started);
        assert!(!marker.exists());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_owner_keeps_posix_cmd_and_explicit_zsh_semantics_distinct() {
        let fixture_root = std::env::current_dir().unwrap();
        let workspace = tempfile::tempdir_in(&fixture_root).unwrap();
        let owner =
            ManagedEngineProcessOwner::new(workspace.path(), SupervisorConfig::default()).unwrap();

        let sh = owner
            .execute(
                EngineProcessRequest::shell("printf 'sh:%s' \"${ZSH_VERSION-unset}\""),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            exit_code: Some(0),
            output: sh_output,
            cleanup: sh_cleanup,
            ..
        } = sh
        else {
            panic!("the product cmd transport should execute through /bin/sh");
        };
        assert!(sh_cleanup.reaped);
        assert_eq!(sh_output.text, "sh:unset");

        let mut zsh = EngineProcessRequest::pipe("/bin/zsh");
        zsh.args = vec![
            "-lc".to_owned(),
            "printf 'zsh:%s' \"$ZSH_VERSION\"".to_owned(),
        ];
        let zsh = owner
            .execute(zsh, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            exit_code: Some(0),
            output: zsh_output,
            cleanup: zsh_cleanup,
            ..
        } = zsh
        else {
            panic!("an explicit /bin/zsh invocation should retain zsh semantics");
        };
        assert!(zsh_cleanup.reaped);
        assert!(zsh_output.text.starts_with("zsh:"));
        assert_ne!(zsh_output.text, "zsh:");
    }

    #[tokio::test]
    async fn managed_owner_supports_stdin_and_close() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(stdin_request(), CancellationToken::new())
            .await
            .unwrap();
        owner
            .write_stdin(&session, b"from-stdin\n", CancellationToken::new())
            .await
            .unwrap();
        owner.close_stdin(&session).await.unwrap();
        let outcome = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited { output, .. } = outcome else {
            panic!("stdin fixture should exit normally");
        };
        assert!(output.text.contains("from-stdin"));
    }

    #[tokio::test]
    async fn explicit_poll_cursor_replays_retained_terminal_output() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(echo_request(), CancellationToken::new())
            .await
            .unwrap();
        let first = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            output: first_output,
            ..
        } = first
        else {
            panic!("echo command should exit normally");
        };
        assert!(session.cursor() > 0);

        let replay = owner
            .poll_from(
                &mut session,
                0,
                Duration::ZERO,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            output: replayed_output,
            ..
        } = replay
        else {
            panic!("a terminal poll from cursor zero should replay the outcome");
        };
        assert_eq!(replayed_output.text, first_output.text);
        assert_eq!(replayed_output.next_cursor, first_output.next_cursor);
    }

    #[tokio::test]
    async fn stdin_write_limit_rejects_oversize_without_partial_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(stdin_count_request(), CancellationToken::new())
            .await
            .unwrap();
        let error = owner
            .write_stdin(
                &session,
                &vec![b'x'; 1024 * 1024 + 1],
                CancellationToken::new(),
            )
            .await
            .expect_err("an oversized stdin write must be rejected before delivery");
        assert!(error.to_string().contains("may not exceed 1 MiB"));
        owner
            .write_stdin(&session, b"safe", CancellationToken::new())
            .await
            .unwrap();
        owner.close_stdin(&session).await.unwrap();
        let outcome = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited { output, .. } = outcome else {
            panic!("stdin counter should exit normally");
        };
        assert_eq!(output.text.trim(), "4");
    }

    #[tokio::test]
    async fn stdin_write_accepts_the_exact_one_mib_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(stdin_count_request(), CancellationToken::new())
            .await
            .unwrap();
        owner
            .write_stdin(
                &session,
                &vec![b'x'; 1024 * 1024],
                CancellationToken::new(),
            )
            .await
            .unwrap();
        owner.close_stdin(&session).await.unwrap();
        let outcome = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited { output, .. } = outcome else {
            panic!("stdin counter should exit normally");
        };
        assert_eq!(output.text.trim(), (1024 * 1024).to_string());
    }

    #[tokio::test]
    async fn terminal_session_rejects_late_stdin_without_changing_replay() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(echo_request(), CancellationToken::new())
            .await
            .unwrap();
        let first = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            output: first_output,
            ..
        } = first
        else {
            panic!("echo command should exit normally");
        };

        owner
            .write_stdin(&session, b"late", CancellationToken::new())
            .await
            .expect_err("terminal stdin must reject instead of reviving the process");
        let replay = owner
            .poll_from(
                &mut session,
                0,
                Duration::ZERO,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let EngineProcessPoll::Exited {
            output: replayed_output,
            ..
        } = replay
        else {
            panic!("terminal replay must remain exited");
        };
        assert_eq!(replayed_output, first_output);
    }

    #[tokio::test]
    async fn cancellation_returns_a_reaped_terminal_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(sleeper_request(), CancellationToken::new())
            .await
            .unwrap();
        let outcome = owner.cancel(&mut session).await.unwrap();
        let EngineProcessPoll::Cancelled { cleanup, .. } = outcome else {
            panic!("sleeping process should be cancelled");
        };
        assert!(cleanup.reaped);
    }

    #[tokio::test]
    async fn deadline_returns_a_reaped_timeout_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut request = sleeper_request();
        // Windows job adoption can take longer than 100 ms under a parallel
        // test load; the deadline must exercise the running child, not cancel
        // the start future before it has returned a session.
        request.timeout_ms = 1_000;
        let outcome = owner
            .execute(request, CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::TimedOut { cleanup, .. } = outcome else {
            panic!("deadline should produce a TimedOut outcome");
        };
        assert!(cleanup.reaped);
    }

    #[tokio::test]
    async fn long_output_is_bounded_and_reports_dropped_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedEngineProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let outcome = owner
            .execute(long_output_request(), CancellationToken::new())
            .await
            .unwrap();
        let EngineProcessPoll::Exited { output, .. } = outcome else {
            panic!("long-output fixture should exit normally");
        };
        assert!(output.retained_bytes <= 256);
        assert!(output.dropped_bytes > 0);
        assert!(output.text.len() <= 256);
    }
}
