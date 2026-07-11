//! Legacy `exec_command` schema backed by the shared process supervisor.
//!
//! The model-visible numeric `session_id` remains unchanged in Wave A. It is a
//! short adapter to an owner-qualified UUIDv7 supervisor session; no process or
//! PTY object is retained in this crate.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use nomi_execution::{
    CapabilityPolicy, CommandSpec, ExecutionError, ExecutionOutcome, ExecutionOwner,
    ExecutionPolicy, OutputSnapshot, OutputStream, PollResult, ProcessSupervisor, ShellKind,
    Transport, normalize_request,
};
use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    Tool,
    process_store::{LegacySessionBinding, ProcessStore, missed_bytes},
};

const DEFAULT_YIELD_MS: u64 = 10_000;
const MIN_YIELD_MS: u64 = 250;
const MAX_YIELD_MS: u64 = 30_000;
const TERMINAL_SETTLE_MS: u64 = 25;
const PTY_COLS: u16 = 120;
const PTY_ROWS: u16 = 30;

pub struct ExecCommandTool {
    supervisor: Arc<ProcessSupervisor>,
    store: Arc<ProcessStore>,
    default_cwd: PathBuf,
    capability: CapabilityPolicy,
    run_id: Uuid,
}

impl ExecCommandTool {
    pub fn new(
        supervisor: Arc<ProcessSupervisor>,
        store: Arc<ProcessStore>,
        cwd: PathBuf,
        capability: CapabilityPolicy,
    ) -> Self {
        Self {
            supervisor,
            store,
            default_cwd: cwd,
            capability,
            run_id: Uuid::now_v7(),
        }
    }
}

#[async_trait]
impl Tool for ExecCommandTool {
    fn name(&self) -> &str {
        "exec_command"
    }

    fn description(&self) -> &str {
        "Runs a command with supervised pipe or PTY transport, returning its output or a numeric \
         session_id for ongoing interaction.\n\n\
         The command is executed by the platform shell. On Windows this is PowerShell \
         (use PowerShell syntax such as Get-ChildItem, $env:NAME, and ';' for sequencing; \
         run cmd /C \"...\" explicitly when cmd.exe syntax is required). On macOS/Linux this \
         is POSIX sh.\n\n\
         Use tty=true for REPLs, TUIs, and interactive installers.\n\n\
         - tty=false uses separate stdout/stderr pipe streams.\n\
         - tty=true uses a merged PTY stream for interactive programs.\n\
         - If the process exits within yield-time_ms, the result reports its exit_code and no \
         session_id.\n\
         - If it remains live, use write_stdin with the returned session_id.\n\n\
         IMPORTANT (TUI submit): send the line of text first, then send the Enter/return key \
         (\"\\r\") as its own write_stdin call. A TUI may treat text plus return in one burst as \
         pasted input and leave the command unsubmitted."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "cmd": {
                    "type": "string",
                    "description": "The shell command to execute."
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory. Defaults to the session cwd."
                },
                "tty": {
                    "type": "boolean",
                    "description": "Use PTY transport. Defaults to false (pipe)."
                },
                "yield_time_ms": {
                    "type": "number",
                    "description": "Milliseconds to wait before yielding. Default 10000, range 250-30000."
                }
            },
            "required": ["cmd"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    fn describe(&self, input: &Value) -> String {
        let command = input.get("cmd").and_then(Value::as_str).unwrap_or("");
        format!("exec_command: {}", crate::truncate_utf8(command, 80))
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let command = match input.get("cmd").and_then(Value::as_str) {
            Some(command) if !command.is_empty() => command.to_owned(),
            _ => return ToolResult::error("exec_command: missing required parameter `cmd`"),
        };
        let cwd = match requested_workdir(&input, &self.default_cwd) {
            Ok(cwd) => cwd,
            Err(error) => return ToolResult::error(format!("exec_command: {error}")),
        };
        let tty = input.get("tty").and_then(Value::as_bool).unwrap_or(false);
        let transport = if tty {
            Transport::Pty {
                cols: PTY_COLS,
                rows: PTY_ROWS,
            }
        } else {
            Transport::Pipe
        };
        let yield_ms = input
            .get("yield_time_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_YIELD_MS)
            .clamp(MIN_YIELD_MS, MAX_YIELD_MS);
        prune_stale_bindings(&self.supervisor, &self.store);
        let owner = ExecutionOwner::new(self.run_id, Uuid::now_v7());
        let request = nomi_execution::ExecutionRequest {
            owner,
            command: CommandSpec::Shell {
                shell: if cfg!(windows) {
                    ShellKind::PowerShell
                } else {
                    ShellKind::Posix
                },
                script: command,
            },
            cwd: cwd.clone(),
            env: Default::default(),
            transport,
            policy: ExecutionPolicy::default(),
            capability: self.capability.clone(),
        };
        let request = match normalize_request(request, &self.default_cwd) {
            Ok(request) => request,
            Err(error) => return execution_error("prepare", error),
        };
        let handle = match self.supervisor.start(request).await {
            Ok(handle) => handle,
            Err(error) => return execution_error("start", error),
        };
        let mut guard = StartedSessionGuard::new(
            Arc::clone(&self.supervisor),
            handle.owner.clone(),
            handle.session_id,
        );
        let poll = self
            .supervisor
            .poll(
                &handle.owner,
                &handle.session_id,
                nomi_execution::OutputCursor::START,
                Instant::now() + Duration::from_millis(yield_ms),
            )
            .await;
        let poll = match poll {
            Ok(poll) => poll,
            Err(error) => return execution_error("poll", error),
        };
        match poll {
            PollResult::Finished(outcome) => {
                guard.disarm();
                render_terminal(outcome, transport)
            }
            PollResult::Running {
                output: initial_output,
                ..
            } => {
                let settled = self
                    .supervisor
                    .poll_until_activity(
                        &handle.owner,
                        &handle.session_id,
                        initial_output.next_cursor,
                        Instant::now() + Duration::from_millis(TERMINAL_SETTLE_MS),
                    )
                    .await;
                match settled {
                    Ok(PollResult::Finished(outcome)) => {
                        guard.disarm();
                        return render_terminal(outcome, transport);
                    }
                    Ok(PollResult::Running { .. }) => {}
                    Err(error) => return execution_error("settle poll", error),
                }
                let output = match self
                    .supervisor
                    .poll_until_activity(
                        &handle.owner,
                        &handle.session_id,
                        nomi_execution::OutputCursor::START,
                        Instant::now(),
                    )
                    .await
                {
                    Ok(PollResult::Finished(outcome)) => {
                        guard.disarm();
                        return render_terminal(outcome, transport);
                    }
                    Ok(PollResult::Running { output, .. }) => output,
                    Err(error) => return execution_error("snapshot poll", error),
                };
                let binding = LegacySessionBinding::after_output(
                    handle.owner.clone(),
                    handle.session_id,
                    transport,
                    &output,
                );
                let id = match self.store.insert(binding) {
                    Ok(id) => id,
                    Err(error) => {
                        let cleanup = self
                            .supervisor
                            .terminate(&handle.owner, &handle.session_id)
                            .await;
                        guard.disarm();
                        return ToolResult::error(format!(
                            "exec_command: could not retain the live session: {error}; cleanup={}",
                            cleanup
                                .as_ref()
                                .map(outcome_summary)
                                .unwrap_or_else(|error| error.to_string())
                        ));
                    }
                };
                guard.disarm();
                ToolResult::text(format!(
                    "session_id={id}\ntransport={}\n(process still running — use write_stdin to continue)\n{}",
                    transport_label(transport),
                    render_output(&output, Some(missed_bytes(&output, nomi_execution::OutputCursor::START)))
                ))
            }
        }
    }
}

fn prune_stale_bindings(supervisor: &ProcessSupervisor, store: &ProcessStore) {
    for (id, entry) in store.entries() {
        // A ready terminal outcome may still contain unread tail output for the
        // next write_stdin poll, so retain that mapping. Only identities the
        // supervisor has already retired (or cannot authenticate) are stale.
        match supervisor
            .terminal_outcome_if_ready(
                entry.owner(),
                &entry.session_id(),
                nomi_execution::OutputCursor::START,
            )
        {
            Err(ExecutionError::SessionNotFound { .. })
            | Err(ExecutionError::OwnerMismatch { .. }) => {
                store.remove_if_same(id, &entry);
            }
            Ok(Some(_)) | Ok(None) | Err(_) => {}
        }
    }
}

fn requested_workdir(input: &Value, default: &Path) -> Result<PathBuf, &'static str> {
    match input.get("workdir") {
        None | Some(Value::Null) => Ok(default.to_path_buf()),
        Some(Value::String(value)) if value.is_empty() => Err("workdir must not be empty"),
        Some(Value::String(value)) => Ok(PathBuf::from(value)),
        Some(_) => Err("workdir must be a string"),
    }
}

pub(crate) fn render_output(
    output: &OutputSnapshot,
    missed_bytes: Option<u64>,
) -> String {
    let mut chunks = output.chunks.iter().collect::<Vec<_>>();
    chunks.sort_by_key(|chunk| chunk.seq);
    let mut rendered = String::new();
    let mut current_stream = None;
    for chunk in chunks {
        if current_stream != Some(chunk.stream) {
            if !rendered.is_empty() && !rendered.ends_with('\n') {
                rendered.push('\n');
            }
            rendered.push_str(match chunk.stream {
                OutputStream::Stdout => "STDOUT:\n",
                OutputStream::Stderr => "STDERR:\n",
                OutputStream::Pty => "PTY:\n",
            });
            current_stream = Some(chunk.stream);
        }
        rendered.push_str(&chunk.text);
    }
    if rendered.is_empty() {
        rendered.push_str("OUTPUT:\n");
    }
    let missed_bytes = missed_bytes.unwrap_or(0);
    if missed_bytes > 0
        || output.dropped_bytes > 0
        || output.encoding.decode_errors > 0
        || output.encoding.source_encoding != "utf-8"
    {
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str(&format!(
            "[output metadata: missed_bytes={missed_bytes}, dropped_bytes={}, source_encoding={}, decode_errors={}]",
            output.dropped_bytes,
            output.encoding.source_encoding,
            output.encoding.decode_errors
        ));
    }
    rendered
}

pub(crate) fn render_terminal(
    outcome: ExecutionOutcome,
    transport: Transport,
) -> ToolResult {
    render_terminal_with_missed(outcome, transport, None)
}

pub(crate) fn render_terminal_with_missed(
    outcome: ExecutionOutcome,
    transport: Transport,
    missed_bytes: Option<u64>,
) -> ToolResult {
    match outcome {
        ExecutionOutcome::Exited {
            code,
            signal,
            output,
            cleanup,
        } => {
            let exit_code = code.unwrap_or(-1);
            let mut content = format!(
                "(process exited, exit_code={exit_code})\ntransport={}\n{}",
                transport_label(transport),
                render_output(&output, missed_bytes)
            );
            if let Some(signal) = signal {
                content.push_str(&format!("\nsignal={signal}"));
            }
            append_cleanup(&mut content, &cleanup);
            ToolResult {
                content,
                is_error: code != Some(0) || signal.is_some(),
                images: Vec::new(),
            }
        }
        ExecutionOutcome::Cancelled { output, cleanup } => {
            let mut content = format!(
                "(process cancelled)\ntransport={}\n{}",
                transport_label(transport),
                render_output(&output, missed_bytes)
            );
            append_cleanup(&mut content, &cleanup);
            ToolResult::error(content)
        }
        ExecutionOutcome::TimedOut { output, cleanup } => {
            let mut content = format!(
                "(process timed out)\ntransport={}\n{}",
                transport_label(transport),
                render_output(&output, missed_bytes)
            );
            append_cleanup(&mut content, &cleanup);
            ToolResult::error(content)
        }
        ExecutionOutcome::Lost {
            last_known,
            cleanup,
        } => {
            let mut content = format!(
                "(process lost, pid={}, state={:?})\ntransport={}",
                last_known.pid,
                last_known.state,
                transport_label(transport)
            );
            append_cleanup(&mut content, &cleanup);
            ToolResult::error(content)
        }
        ExecutionOutcome::SpawnFailed(failure) => ToolResult::error(format!(
            "exec_command: spawn failed: {} ({})",
            failure.message, failure.code
        )),
    }
}

pub(crate) fn transport_label(transport: Transport) -> &'static str {
    match transport {
        Transport::Pipe => "pipe",
        Transport::Pty { .. } => "pty",
    }
}

pub(crate) fn outcome_summary(outcome: &ExecutionOutcome) -> String {
    match outcome {
        ExecutionOutcome::Exited { code, signal, .. } => {
            format!("exited code={code:?} signal={signal:?}")
        }
        ExecutionOutcome::Cancelled { cleanup, .. } => {
            format!("cancelled reaped={}", cleanup.reaped)
        }
        ExecutionOutcome::TimedOut { cleanup, .. } => {
            format!("timed_out reaped={}", cleanup.reaped)
        }
        ExecutionOutcome::Lost {
            last_known,
            cleanup,
        } => format!(
            "lost pid={} reaped={} errors={}",
            last_known.pid,
            cleanup.reaped,
            cleanup.errors.join("; ")
        ),
        ExecutionOutcome::SpawnFailed(failure) => {
            format!("spawn_failed {}: {}", failure.code, failure.message)
        }
    }
}

fn append_cleanup(content: &mut String, cleanup: &nomi_execution::CleanupReport) {
    if !cleanup.errors.is_empty() {
        content.push_str("\ncleanup diagnostics: ");
        content.push_str(&cleanup.errors.join("; "));
    }
}

fn execution_error(operation: &str, error: ExecutionError) -> ToolResult {
    ToolResult::error(format!(
        "exec_command: {operation} failed: {error} ({})",
        error.code()
    ))
}

struct StartedSessionGuard {
    supervisor: Arc<ProcessSupervisor>,
    owner: ExecutionOwner,
    session_id: nomi_execution::SessionId,
    armed: bool,
}

impl StartedSessionGuard {
    fn new(
        supervisor: Arc<ProcessSupervisor>,
        owner: ExecutionOwner,
        session_id: nomi_execution::SessionId,
    ) -> Self {
        Self {
            supervisor,
            owner,
            session_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StartedSessionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let supervisor = Arc::clone(&self.supervisor);
        let owner = self.owner.clone();
        let session_id = self.session_id;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = supervisor.terminate(&owner, &session_id).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::pty_test_helper_shell_cmd;

    fn tool(cwd: PathBuf) -> (ExecCommandTool, Arc<ProcessStore>) {
        let supervisor = ProcessSupervisor::new(nomi_execution::SupervisorConfig::default());
        let store = Arc::new(ProcessStore::new());
        (
            ExecCommandTool::new(
                supervisor,
                Arc::clone(&store),
                cwd.clone(),
                CapabilityPolicy::local_owner(cwd),
            ),
            store,
        )
    }

    fn parse_session_id(content: &str) -> Option<u64> {
        content
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .and_then(|value| value.trim().parse().ok())
    }

    fn stdout_stderr_command() -> &'static str {
        if cfg!(windows) {
            "[Console]::Out.WriteLine('pipe_stdout_marker'); [Console]::Error.WriteLine('pipe_stderr_marker')"
        } else {
            "printf 'pipe_stdout_marker\\n'; printf 'pipe_stderr_marker\\n' >&2"
        }
    }

    fn assert_marker_stream(content: &str, marker: &str, expected: &str) {
        let marker_index = content.find(marker).expect("marker");
        let prefix = &content[..marker_index];
        let stdout = prefix.rfind("STDOUT:\n");
        let stderr = prefix.rfind("STDERR:\n");
        let actual = match (stdout, stderr) {
            (Some(left), Some(right)) if left > right => "STDOUT:\n",
            (Some(_), Some(_)) => "STDERR:\n",
            (Some(_), None) => "STDOUT:\n",
            (None, Some(_)) => "STDERR:\n",
            (None, None) => panic!("missing stream label: {content}"),
        };
        assert_eq!(actual, expected);
    }

    async fn execute_in_workdir(default_cwd: PathBuf, workdir: &str) -> ToolResult {
        tool(default_cwd)
            .0
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("exit 0"),
                "workdir": workdir,
                "tty": false,
                "yield_time_ms": 250
            }))
            .await
    }

    #[tokio::test]
    async fn immediate_exit_reports_exit_code_no_session() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("exit 0"),
                "yield-time_ms": 3000
            }))
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("exit_code=0"));
        assert!(parse_session_id(&result.content).is_none());
    }

    #[tokio::test]
    async fn immediate_exit_does_not_wait_for_the_remaining_yield() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let started = Instant::now();
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("exit 0"),
                "yield_time_ms": 3000
            }))
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn tty_false_quick_command_keeps_both_streams_through_terminal_exit() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": stdout_stderr_command(),
                "tty": false,
                "yield-time_ms": 10_000
            }))
            .await;

        assert!(
            !result.is_error && parse_session_id(&result.content).is_none(),
            "{}",
            result.content
        );
        assert_marker_stream(&result.content, "pipe_stdout_marker", "STDOUT:\n");
        assert_marker_stream(&result.content, "pipe_stderr_marker", "STDERR:\n");
    }

    #[tokio::test]
    async fn tty_true_reports_pty_transport() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("exit 0"),
                "tty": true,
                "yield_time_ms": 1000
            }))
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("transport=pty"));
    }

    #[tokio::test]
    async fn exit_seven_is_an_error() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("exit 7"),
                "yield_time_ms": 3000
            }))
            .await;
        assert!(result.is_error, "{}", result.content);
        assert!(result.content.contains("exit_code=7"), "{}", result.content);
    }

    #[tokio::test]
    async fn long_lived_returns_a_numeric_adapter_session() {
        let (tool, store) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("echo-stdin"),
                "yield_time_ms": 400
            }))
            .await;
        let id = parse_session_id(&result.content).expect("session id");
        assert!(store.contains(id));
    }

    #[tokio::test]
    async fn settle_poll_preserves_output_from_before_and_during_settle() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd(
                    "emit-twice 200 first_marker 20 second_marker 60000"
                ),
                "yield-time_ms": 250
            }))
            .await;

        assert!(result.content.contains("first_marker"), "{}", result.content);
        assert!(result.content.contains("second_marker"), "{}", result.content);
    }

    #[tokio::test]
    async fn output_before_exit_still_returns_terminal_within_the_initial_yield() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let result = tool
            .execute(json!({
                "cmd": pty_test_helper_shell_cmd("emit-after 50 before_exit 300"),
                "yield-time_ms": 1000
            }))
            .await;

        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("before_exit"), "{}", result.content);
        assert!(result.content.contains("exit_code=0"), "{}", result.content);
        assert!(parse_session_id(&result.content).is_none());
    }

    #[test]
    fn initial_output_renderer_reports_exact_missed_bytes() {
        let output = OutputSnapshot {
            chunks: Vec::new(),
            next_cursor: nomi_execution::OutputCursor::new(4 * 1024 * 1024 + 17),
            retained_bytes: 4 * 1024 * 1024,
            dropped_bytes: 17,
            encoding: nomi_execution::EncodingMetadata::default(),
        };
        let rendered = render_output(
            &output,
            Some(missed_bytes(
                &output,
                nomi_execution::OutputCursor::START,
            )),
        );

        assert!(rendered.contains("missed_bytes=17"), "{rendered}");
        assert!(rendered.contains("dropped_bytes=17"), "{rendered}");
    }

    #[tokio::test]
    async fn invalid_workdirs_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let file = root.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        let outside = tempfile::tempdir().unwrap();
        for workdir in [
            "",
            missing.to_str().unwrap(),
            file.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ] {
            let result = execute_in_workdir(root.path().to_path_buf(), workdir).await;
            assert!(result.is_error, "workdir={workdir}: {}", result.content);
        }
    }

    #[tokio::test]
    async fn missing_cmd_is_error() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        assert!(tool.execute(json!({})).await.is_error);
    }

    #[test]
    fn description_preserves_shell_and_tui_guidance() {
        let (tool, _) = tool(std::env::current_dir().unwrap());
        let description = tool.description();

        assert!(description.contains("Get-ChildItem"));
        assert!(description.contains("$env:NAME"));
        assert!(description.contains("cmd /C"));
        assert!(description.contains("\"\\r\") as its own write_stdin call"));
    }
}
