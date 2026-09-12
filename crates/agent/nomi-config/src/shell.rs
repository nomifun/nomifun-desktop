use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use nomi_process_runtime::{
    CapabilityPolicy, CommandSpec, OutputCursor, OutputSnapshot, OutputStream, PollResult,
    ProcessOutcome, ProcessOwner, ProcessPolicy, ProcessRequest, ProcessSupervisor, ShellKind,
    Transport, normalize_request,
};
use uuid::Uuid;


#[derive(Clone)]
pub struct SupervisedShell {
    supervisor: Arc<ProcessSupervisor>,
    capability: CapabilityPolicy,
    invocation_id: Uuid,
}

#[derive(Debug)]
pub struct SupervisedShellOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisedShellError {
    #[error("shell process failed: {0}")]
    Process(#[from] nomi_process_runtime::ProcessError),
    #[error("shell process timed out after {timeout_ms}ms")]
    TimedOut { timeout_ms: u64 },
    #[error("shell process was cancelled before completion")]
    Cancelled,
    #[error("shell process ownership was lost: {0}")]
    Lost(String),
    #[error("shell process failed to spawn: {0}")]
    SpawnFailed(String),
}

impl SupervisedShell {
    pub fn new(supervisor: Arc<ProcessSupervisor>, cwd_root: PathBuf) -> Self {
        Self {
            supervisor,
            capability: CapabilityPolicy::local_owner(cwd_root),
            invocation_id: Uuid::now_v7(),
        }
    }

    pub fn standalone(cwd_root: PathBuf) -> Self {
        Self::new(
            ProcessSupervisor::new(nomi_process_runtime::SupervisorConfig::default()),
            cwd_root,
        )
    }


    /// Execute one shell command under the shared exact process-tree
    /// supervisor. A returned success/error is emitted only after the complete
    /// process tree has been reaped. If the caller future is cancelled, the
    /// registered session remains owned by the supervisor and is drained by
    /// the turn's `ProcessSupervisor::quiesce` fence.
    pub async fn output(
        &self,
        command: &str,
        cwd: &Path,
        env: &HashMap<String, String>,
        timeout: Option<Duration>,
    ) -> Result<SupervisedShellOutput, SupervisedShellError> {
        let started_at = Instant::now();
        let deadline = timeout.and_then(|duration| started_at.checked_add(duration));
        let request = ProcessRequest {
            owner: ProcessOwner::new(self.invocation_id, Uuid::now_v7()),
            command: CommandSpec::Shell {
                shell: if cfg!(windows) {
                    ShellKind::PowerShell
                } else {
                    ShellKind::Posix
                },
                script: command.to_owned(),
            },
            cwd: cwd.to_path_buf(),
            env: env
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value)))
                .collect::<BTreeMap<_, _>>(),
            transport: Transport::Pipe,
            policy: ProcessPolicy {
                deadline,
                ..ProcessPolicy::default()
            },
            capability: self.capability.clone(),
        };
        let request = normalize_request(request, cwd)?;
        let handle = self.supervisor.start(request).await?;

        let outcome = loop {
            let yield_until = deadline.unwrap_or_else(|| {
                Instant::now()
                    .checked_add(Duration::from_secs(60 * 60))
                    .unwrap_or_else(Instant::now)
            });
            match self
                .supervisor
                .poll(
                    &handle.owner,
                    &handle.session_id,
                    OutputCursor::START,
                    yield_until,
                )
                .await
            {
                Ok(PollResult::Finished(outcome)) => break outcome,
                Ok(PollResult::Running { .. })
                    if deadline.is_some_and(|deadline| Instant::now() >= deadline) =>
                {
                    break self
                        .supervisor
                        .timeout(&handle.owner, &handle.session_id)
                        .await?;
                }
                Ok(PollResult::Running { .. }) => {}
                Err(error) => {
                    let _ = self
                        .supervisor
                        .cancel(&handle.owner, &handle.session_id)
                        .await;
                    return Err(error.into());
                }
            }
        };

        match outcome {
            ProcessOutcome::Exited {
                code,
                signal,
                output,
                ..
            } => Ok(SupervisedShellOutput {
                success: code == Some(0) && signal.is_none(),
                code,
                stdout: output_stream_text(&output, OutputStream::Stdout),
                stderr: output_stream_text(&output, OutputStream::Stderr),
            }),
            ProcessOutcome::TimedOut { .. } => Err(SupervisedShellError::TimedOut {
                timeout_ms: timeout
                    .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                    .unwrap_or_default(),
            }),
            ProcessOutcome::Cancelled { .. } => Err(SupervisedShellError::Cancelled),
            ProcessOutcome::Lost { cleanup, .. } => {
                Err(SupervisedShellError::Lost(cleanup.errors.join("; ")))
            }
            ProcessOutcome::SpawnFailed(failure) => Err(SupervisedShellError::SpawnFailed(
                format!("{}: {}", failure.code, failure.message),
            )),
        }
    }
}

fn output_stream_text(output: &OutputSnapshot, stream: OutputStream) -> String {
    output
        .chunks
        .iter()
        .filter(|chunk| chunk.stream == stream)
        .map(|chunk| chunk.text.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervised_shell_passes_environment_and_cwd() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("proof.txt"), "local-file").unwrap();
        let shell = SupervisedShell::standalone(temp.path().to_path_buf());
        let command = if cfg!(windows) {
            "Write-Output $env:NOMI_TEST_VALUE; Get-Content proof.txt"
        } else {
            "printf '%s\\n' \"$NOMI_TEST_VALUE\"; cat proof.txt"
        };
        let env = HashMap::from([("NOMI_TEST_VALUE".into(), "test-value".into())]);
        let output = shell.output(command, temp.path(), &env, Some(Duration::from_secs(10))).await.unwrap();
        assert!(output.success, "{}", output.stderr);
        assert!(output.stdout.contains("test-value"));
        assert!(output.stdout.contains("local-file"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn supervised_shell_accepts_powershell_syntax() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("proof.txt"), "ok").unwrap();
        let shell = SupervisedShell::standalone(temp.path().to_path_buf());
        let output = shell.output(
            "if (Test-Path proof.txt) { Get-Content proof.txt } else { exit 9 }",
            temp.path(), &HashMap::new(), Some(Duration::from_secs(10)),
        ).await.unwrap();
        assert!(output.success, "{}", output.stderr);
        assert_eq!(output.stdout.trim(), "ok");
    }

    #[tokio::test]
    async fn supervised_shell_preserves_nonzero_exit_code() {
        let temp = tempfile::tempdir().unwrap();
        let shell = SupervisedShell::standalone(temp.path().to_path_buf());
        let command = if cfg!(windows) { "cmd /c exit 7" } else { "exit 7" };
        let output = shell.output(command, temp.path(), &HashMap::new(), Some(Duration::from_secs(10))).await.unwrap();
        assert!(!output.success);
        assert_eq!(output.code, Some(7));
    }
}
