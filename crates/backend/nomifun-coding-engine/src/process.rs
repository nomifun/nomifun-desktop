//! Reusable Coding owner adapter for `nomi-process-runtime`.
//!
//! This module does not expose a model Tool and is not called directly by the
//! Coding turn loop. The application-owned Wave2/Kernel host adapter may use
//! it to implement `process.exec` and interactive process-session actions
//! without duplicating process ownership.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nomi_process_runtime::{
    CapabilityPolicy, CleanupReport, CommandSpec, EncodingMetadata, NormalizedProcessRequest,
    OutputCursor, OutputSnapshot, PollResult, ProcessError, ProcessOutcome, ProcessOwner,
    ProcessPolicy, ProcessRequest, ProcessSupervisor, SandboxPolicy, SessionId,
    SupervisorConfig, Transport, normalize_request,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::CodingEngineError;

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
pub enum CodingProcessTransport {
    Pipe,
    Pty { cols: u16, rows: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingProcessRequest {
    pub command: String,
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
    pub transport: CodingProcessTransport,
}

impl CodingProcessRequest {
    pub fn pipe(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            output_limit_bytes: DEFAULT_OUTPUT_LIMIT_BYTES,
            transport: CodingProcessTransport::Pipe,
        }
    }

    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if self.command.trim().is_empty()
            || self.command.trim() != self.command
            || self.command.contains('\0')
            || self.command.chars().count() > MAX_COMMAND_CHARS
        {
            return Err(CodingEngineError::Process(
                "command must be a bounded executable without edge whitespace or NUL bytes"
                    .to_owned(),
            ));
        }
        if self.args.len() > MAX_ARGUMENTS
            || self.args.iter().any(|argument| {
                argument.contains('\0') || argument.chars().count() > MAX_ARGUMENT_CHARS
            })
        {
            return Err(CodingEngineError::Process(
                "process arguments exceed the Coding process limits".to_owned(),
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
            return Err(CodingEngineError::Process(
                "process environment contains an invalid or oversized entry".to_owned(),
            ));
        }
        if !(1..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(CodingEngineError::Process(format!(
                "timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"
            )));
        }
        if self.output_limit_bytes == 0 || self.output_limit_bytes > MAX_OUTPUT_LIMIT_BYTES {
            return Err(CodingEngineError::Process(format!(
                "output_limit_bytes must be between 1 and {MAX_OUTPUT_LIMIT_BYTES}"
            )));
        }
        if matches!(
            self.transport,
            CodingProcessTransport::Pty { cols: 0, .. }
                | CodingProcessTransport::Pty { rows: 0, .. }
        ) {
            return Err(CodingEngineError::Process(
                "PTY dimensions must be non-zero".to_owned(),
            ));
        }
        if self.cwd.as_deref().is_some_and(invalid_relative_path) {
            return Err(CodingEngineError::Process(
                "cwd must be a normalized workspace-relative path".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CodingProcessSession {
    owner: ProcessOwner,
    session_id: SessionId,
    pid: u32,
    cursor: OutputCursor,
}

impl CodingProcessSession {
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
pub struct CodingProcessOutput {
    pub text: String,
    pub next_cursor: u64,
    pub retained_bytes: usize,
    pub dropped_bytes: u64,
    pub source_encoding: String,
    pub decode_errors: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodingProcessPoll {
    Running {
        pid: u32,
        output: CodingProcessOutput,
    },
    Exited {
        exit_code: Option<i32>,
        signal: Option<i32>,
        output: CodingProcessOutput,
        cleanup: CodingCleanupReport,
    },
    Cancelled {
        output: CodingProcessOutput,
        cleanup: CodingCleanupReport,
    },
    TimedOut {
        output: CodingProcessOutput,
        cleanup: CodingCleanupReport,
    },
    Lost {
        pid: u32,
        output: CodingProcessOutput,
        cleanup: CodingCleanupReport,
    },
    SpawnFailed {
        code: String,
        message: String,
    },
}

impl CodingProcessPoll {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCleanupReport {
    pub interrupt_attempted: bool,
    pub terminate_attempted: bool,
    pub force_kill_attempted: bool,
    pub reaped: bool,
    pub elapsed_ms: u64,
    pub errors: Vec<String>,
}

pub struct ManagedCodingProcessOwner {
    workspace_root: PathBuf,
    supervisor: Arc<ProcessSupervisor>,
}

impl ManagedCodingProcessOwner {
    pub fn new(
        workspace_root: impl AsRef<Path>,
        config: SupervisorConfig,
    ) -> Result<Self, CodingEngineError> {
        let workspace_root = std::fs::canonicalize(workspace_root.as_ref()).map_err(|error| {
            CodingEngineError::Process(format!(
                "workspace root {} is unavailable: {error}",
                workspace_root.as_ref().display()
            ))
        })?;
        if !workspace_root.is_dir() {
            return Err(CodingEngineError::Process(
                "workspace root is not a directory".to_owned(),
            ));
        }
        Ok(Self {
            workspace_root,
            supervisor: ProcessSupervisor::new(config),
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub async fn start(
        &self,
        request: CodingProcessRequest,
        cancellation: CancellationToken,
    ) -> Result<CodingProcessSession, CodingEngineError> {
        request.validate()?;
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let normalized = self.normalized_request(request)?;
        let start = self.supervisor.start(normalized);
        let handle = tokio::select! {
            _ = cancellation.cancelled() => return Err(CodingEngineError::Cancelled),
            result = start => result.map_err(process_error)?,
        };
        Ok(CodingProcessSession {
            owner: handle.owner,
            session_id: handle.session_id,
            pid: handle.pid,
            cursor: OutputCursor::START,
        })
    }

    pub async fn poll(
        &self,
        session: &mut CodingProcessSession,
        wait: Duration,
        cancellation: CancellationToken,
    ) -> Result<CodingProcessPoll, CodingEngineError> {
        if cancellation.is_cancelled() {
            return self.cancel(session).await;
        }
        let wait = wait.min(MAX_POLL_WAIT);
        let poll = self.supervisor.poll_until_activity(
            &session.owner,
            &session.session_id,
            session.cursor,
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
        session: &mut CodingProcessSession,
        cancellation: CancellationToken,
    ) -> Result<CodingProcessPoll, CodingEngineError> {
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
        session: &CodingProcessSession,
        bytes: &[u8],
        cancellation: CancellationToken,
    ) -> Result<(), CodingEngineError> {
        if bytes.len() > 1024 * 1024 {
            return Err(CodingEngineError::Process(
                "one stdin write may not exceed 1 MiB".to_owned(),
            ));
        }
        let write = self
            .supervisor
            .write(&session.owner, &session.session_id, bytes);
        tokio::select! {
            _ = cancellation.cancelled() => Err(CodingEngineError::Cancelled),
            result = write => result.map_err(process_error),
        }
    }

    pub async fn close_stdin(
        &self,
        session: &CodingProcessSession,
    ) -> Result<(), CodingEngineError> {
        self.supervisor
            .close_stdin(&session.owner, &session.session_id)
            .await
            .map_err(process_error)
    }

    pub async fn resize(
        &self,
        session: &CodingProcessSession,
        cols: u16,
        rows: u16,
    ) -> Result<(), CodingEngineError> {
        self.supervisor
            .resize(&session.owner, &session.session_id, cols, rows)
            .await
            .map_err(process_error)
    }

    pub async fn cancel(
        &self,
        session: &mut CodingProcessSession,
    ) -> Result<CodingProcessPoll, CodingEngineError> {
        let outcome = self
            .supervisor
            .cancel(&session.owner, &session.session_id)
            .await
            .map_err(process_error)?;
        Ok(self.convert_outcome(session, outcome))
    }

    fn normalized_request(
        &self,
        request: CodingProcessRequest,
    ) -> Result<NormalizedProcessRequest, CodingEngineError> {
        let timeout = Duration::from_millis(request.timeout_ms);
        let transport = match request.transport {
            CodingProcessTransport::Pipe => Transport::Pipe,
            CodingProcessTransport::Pty { cols, rows } => Transport::Pty { cols, rows },
        };
        let mut policy = ProcessPolicy::default();
        policy.output_limit_bytes = request.output_limit_bytes;
        policy.lease = timeout.saturating_add(Duration::from_secs(60));
        policy.deadline = Some(Instant::now() + timeout);
        let request = ProcessRequest {
            owner: ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7()),
            command: CommandSpec::Program {
                program: OsString::from(request.command),
                args: request.args.into_iter().map(OsString::from).collect(),
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
                sandbox: SandboxPolicy::UnrestrictedLocalOwner,
            },
        };
        normalize_request(request, &self.workspace_root).map_err(process_error)
    }

    fn convert_poll(
        &self,
        session: &mut CodingProcessSession,
        result: PollResult,
    ) -> CodingProcessPoll {
        match result {
            PollResult::Running { snapshot, output } => {
                debug_assert_eq!(snapshot.pid, session.pid);
                session.cursor = output.next_cursor;
                CodingProcessPoll::Running {
                    pid: snapshot.pid,
                    output: convert_output(output),
                }
            }
            PollResult::Finished(outcome) => self.convert_outcome(session, outcome),
        }
    }

    fn convert_outcome(
        &self,
        session: &mut CodingProcessSession,
        outcome: ProcessOutcome,
    ) -> CodingProcessPoll {
        match outcome {
            ProcessOutcome::Exited {
                code,
                signal,
                output,
                cleanup,
            } => {
                session.cursor = output.next_cursor;
                CodingProcessPoll::Exited {
                    exit_code: code,
                    signal,
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::Cancelled { output, cleanup } => {
                session.cursor = output.next_cursor;
                CodingProcessPoll::Cancelled {
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::TimedOut { output, cleanup } => {
                session.cursor = output.next_cursor;
                CodingProcessPoll::TimedOut {
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
                CodingProcessPoll::Lost {
                    pid: last_known.pid,
                    output: convert_output(output),
                    cleanup: convert_cleanup(cleanup),
                }
            }
            ProcessOutcome::SpawnFailed(failure) => CodingProcessPoll::SpawnFailed {
                code: failure.code,
                message: failure.message,
            },
        }
    }
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

fn convert_output(output: OutputSnapshot) -> CodingProcessOutput {
    let text = output.text();
    let EncodingMetadata {
        source_encoding,
        decode_errors,
    } = output.encoding;
    CodingProcessOutput {
        text,
        next_cursor: output.next_cursor.offset(),
        retained_bytes: output.retained_bytes,
        dropped_bytes: output.dropped_bytes,
        source_encoding,
        decode_errors,
    }
}

fn convert_cleanup(cleanup: CleanupReport) -> CodingCleanupReport {
    CodingCleanupReport {
        interrupt_attempted: cleanup.interrupt_attempted,
        terminate_attempted: cleanup.terminate_attempted,
        force_kill_attempted: cleanup.force_kill_attempted,
        reaped: cleanup.reaped,
        elapsed_ms: cleanup.elapsed.as_millis().min(u64::MAX as u128) as u64,
        errors: cleanup.errors,
    }
}

fn process_error(error: ProcessError) -> CodingEngineError {
    CodingEngineError::Process(format!("{}: {error}", error.code()))
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

    fn echo_request() -> CodingProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec")
                .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
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
        CodingProcessRequest {
            command,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 10_000,
            output_limit_bytes: 64 * 1024,
            transport: CodingProcessTransport::Pipe,
        }
    }

    fn stdin_request() -> CodingProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec")
                .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
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
        CodingProcessRequest {
            command,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 10_000,
            output_limit_bytes: 64 * 1024,
            transport: CodingProcessTransport::Pipe,
        }
    }

    fn sleeper_request() -> CodingProcessRequest {
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec")
                .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
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
        CodingProcessRequest {
            command,
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 60_000,
            output_limit_bytes: 64 * 1024,
            transport: CodingProcessTransport::Pipe,
        }
    }

    #[tokio::test]
    async fn managed_owner_runs_and_reaps_a_bounded_process() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedCodingProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(echo_request(), CancellationToken::new())
            .await
            .unwrap();
        let outcome = owner
            .wait(&mut session, CancellationToken::new())
            .await
            .unwrap();
        let CodingProcessPoll::Exited {
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
    async fn workspace_escape_is_rejected_before_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedCodingProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut request = echo_request();
        request.cwd = Some("../outside".to_owned());
        assert!(matches!(
            owner.start(request, CancellationToken::new()).await,
            Err(CodingEngineError::Process(_))
        ));
    }

    #[tokio::test]
    async fn managed_owner_supports_stdin_and_close() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedCodingProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
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
        let CodingProcessPoll::Exited { output, .. } = outcome else {
            panic!("stdin fixture should exit normally");
        };
        assert!(output.text.contains("from-stdin"));
    }

    #[tokio::test]
    async fn cancellation_returns_a_reaped_terminal_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let owner =
            ManagedCodingProcessOwner::new(directory.path(), SupervisorConfig::default()).unwrap();
        let mut session = owner
            .start(sleeper_request(), CancellationToken::new())
            .await
            .unwrap();
        let outcome = owner.cancel(&mut session).await.unwrap();
        let CodingProcessPoll::Cancelled { cleanup, .. } = outcome else {
            panic!("sleeping process should be cancelled");
        };
        assert!(cleanup.reaped);
    }
}
