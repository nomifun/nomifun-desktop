//! Local MCP byte transport. Protocol traffic does not grant platform tools;
//! process placement, tree cleanup and inherited environment remain host-owned.
use std::collections::{BTreeSet, HashMap};
#[cfg(test)]
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout, timeout_at};

use super::{JsonRpcResponse, MAX_RESPONSE_BYTES, McpOwnerError, McpSession, McpSetupPolicy, McpInvocationControl, McpCleanupStatus, PeerTraffic, stream};
use super::control::{SetupBudget, controlled};

pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const EOF_GRACE: Duration = Duration::from_millis(250);
const MAX_STDERR_DIAGNOSTIC_BYTES: usize = 64 * 1024;
pub(super) type Writer = Arc<Mutex<Option<ChildStdin>>>;

struct CancelPendingLaunch(tokio_util::sync::CancellationToken);
impl Drop for CancelPendingLaunch {
    fn drop(&mut self) { self.0.cancel(); }
}

fn uncertain_launch() -> McpOwnerError {
    McpOwnerError::new("MCP_PROCESS_START_UNCERTAIN",
        "MCP process launch was interrupted; platform-owned cleanup has not returned proof")
}

fn accept_launched_process(
    process: ManagedChildProcess, cancellation: &tokio_util::sync::CancellationToken, deadline: Instant,
) -> Result<ManagedChildProcess, McpOwnerError> {
    if cancellation.is_cancelled() || Instant::now() >= deadline {
        // spawn_managed returns only after the platform Job/watchdog owns the
        // exact tree. Drop transfers that authority to its existing cleanup
        // relay (command_builder.rs), never to a raw PID or an unowned task.
        // It is not a cleanup-completion receipt: the caller stays Unproven.
        drop(process);
        return Err(uncertain_launch());
    }
    Ok(process)
}

/// Discovery and execution use the same cumulative setup policy.
pub(crate) async fn discover(
    command: &str, args: &[String], env: &HashMap<String, String>, policy: &McpSetupPolicy,
) -> Result<serde_json::Value, McpOwnerError> {
    let setup = policy.start();
    let control = McpInvocationControl::default();
    let transport = StdioTransport::launch(command, args, env, &setup, &control).await?;
    let mut session = McpSession::from_stdio(transport);
    let cap = if is_package_runner(command, args) { policy.package_bootstrap_timeout } else { policy.protocol_timeout };
    let phase = setup.phase("protocol_setup", cap);
    let result = controlled(async {
        session.initialize().await?;
        let (_, tools) = session.read_catalog(None).await?;
        Ok(serde_json::json!({"tools": tools}))
    }, &control, Some(phase.deadline), || phase.error()).await;
    let outcome = super::finish_session(session, result, &control).await;
    if outcome.cleanup.local == McpCleanupStatus::Unproven {
        return Err(outcome.cleanup.error.expect("unproven cleanup has diagnostic"));
    }
    outcome.result
}

pub(crate) fn is_package_runner(command: &str, args: &[String]) -> bool {
    let basename = command.rsplit(['/', '\\']).next().unwrap_or(command).to_ascii_lowercase();
    let command = [".exe", ".cmd", ".bat"].iter().find_map(|suffix| basename.strip_suffix(suffix)).unwrap_or(&basename);
    match command {
        "npx" | "pnpx" | "bunx" | "uvx" | "pipx" => true,
        "npm" => matches!(args.first().map(String::as_str), Some("exec") | Some("x")),
        "pnpm" => matches!(args.first().map(String::as_str), Some("dlx")),
        "bun" => matches!(args.first().map(String::as_str), Some("x")),
        "uv" => matches!(args.first().map(String::as_str), Some("run") | Some("tool")),
        _ => false,
    }
}

pub(super) struct StdioTransport {
    // Drop transfers unfinished cleanup to the platform relay, not a raw PID
    // kill. Only successful close() is evidence returned to the MCP owner.
    process: ManagedChildProcess,
    stdin: Writer,
    stdout: BufReader<ChildStdout>,
    pending: Vec<u8>,
    traffic: PeerTraffic,
    stderr_diagnostics: Arc<StdMutex<StdioDiagnostics>>,
    stderr_task: Option<JoinHandle<()>>,
}

impl StdioTransport {
    pub(super) async fn launch(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        setup: &SetupBudget,
        control: &McpInvocationControl,
    ) -> Result<Self, McpOwnerError> {
        validate_config(command, args, env)?;
        let cwd = std::env::current_dir().map_err(|cause| McpOwnerError::new(
            "MCP_PROCESS_START_FAILED", format!("MCP launch directory could not be read: {cause}")))?;
        let preparation = setup.phase("runtime_resolution", setup.policy.runtime_preparation_timeout);
        let prepared = controlled(async {
            nomifun_runtime::prepare_process_launch(command, args, &cwd, env).await
                .map_err(|cause| runtime_resolution_error(&cause))
        }, control, Some(preparation.deadline), || preparation.error()).await?;
        let starting = setup.phase("process_start", setup.policy.process_start_timeout);
        let deadline = control.execution_deadline.map_or(starting.deadline, |explicit| starting.deadline.min(explicit));
        let launch_guard = CancelPendingLaunch(control.cancellation.child_token());
        let worker_cancellation = launch_guard.0.clone();
        // Resolution and platform spawn can block. If the waiter is cancelled
        // or times out, the task's result is dropped and ManagedChildProcess
        // still owns cleanup. A timeout while spawn is in progress is never
        // proof that no process started.
        let launch = tokio::task::spawn_blocking(move || {
            if worker_cancellation.is_cancelled() { return Err(super::control::cancelled_error()); }
            if Instant::now() >= deadline { return Err(if deadline == starting.deadline { starting.error() } else { super::control::execution_deadline_error() }); }
            let mut builder = ChildProcessBuilder::new(prepared.program);
            builder
                .env_clear()
                .envs(prepared.environment)
                .args(prepared.args)
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                // stderr is diagnostic-only and is drained into bounded,
                // non-secret categories below. Never merge it into stdout:
                // stdout is the MCP JSON-RPC transport.
                .stderr(Stdio::piped());
            if worker_cancellation.is_cancelled() { return Err(super::control::cancelled_error()); }
            if Instant::now() >= deadline { return Err(if deadline == starting.deadline { starting.error() } else { super::control::execution_deadline_error() }); }
            let process = builder.spawn_managed().map_err(|error| {
                let code = match error.kind() {
                    std::io::ErrorKind::NotFound => "MCP_COMMAND_NOT_FOUND",
                    std::io::ErrorKind::PermissionDenied => "MCP_COMMAND_PERMISSION_DENIED",
                    _ => "MCP_PROCESS_START_UNCERTAIN",
                };
                McpOwnerError::new(code, nomi_redact::redact_secrets_owned(format!("MCP configured stdio process could not be started: {error}")))
            })?;
            let mut process = accept_launched_process(process, &worker_cancellation, deadline)?;
            let stdin = process.child_mut().stdin.take().ok_or_else(uncertain_launch)?;
            let stdout = process.child_mut().stdout.take().ok_or_else(uncertain_launch)?;
            let stderr = process.child_mut().stderr.take().ok_or_else(uncertain_launch)?;
            Ok((process, stdin, stdout, stderr))
        });
        let (process, stdin, stdout, stderr) = tokio::select! {
            biased;
            _ = control.cancellation.cancelled() => return Err(uncertain_launch()),
            result = timeout_at(deadline, launch) => result.map_err(|_| {
                let mut error = uncertain_launch();
                if deadline == starting.deadline { error.setup_timeout = Some((starting.phase, starting.timeout)); }
                error
            })?.map_err(|_| uncertain_launch())??,
        };
        let stderr_diagnostics = Arc::new(StdMutex::new(StdioDiagnostics::default()));
        let stderr_task = tokio::spawn(drain_stderr(stderr, Arc::clone(&stderr_diagnostics)));
        Ok(Self {
            process,
            stdin: Arc::new(Mutex::new(Some(stdin))),
            stdout: BufReader::new(stdout),
            pending: Vec::new(),
            traffic: PeerTraffic::default(),
            stderr_diagnostics,
            stderr_task: Some(stderr_task),
        })
    }

    pub(super) fn writer(&self) -> Writer {
        Arc::clone(&self.stdin)
    }

    async fn next(&mut self) -> Result<String, McpOwnerError> {
        loop {
            let bytes = self.stdout.fill_buf().await.map_err(|_| {
                McpOwnerError::connection_failed("MCP process stdout could not be read")
            })?;
            if bytes.is_empty() {
                return Err(self.closed_stdout_error().await);
            }
            let newline = bytes.iter().position(|byte| *byte == b'\n');
            let content = newline.unwrap_or(bytes.len());
            let consumed = content + usize::from(newline.is_some());
            self.traffic.observe(consumed, false)?;
            if content > MAX_RESPONSE_BYTES.saturating_sub(self.pending.len()) {
                return Err(McpOwnerError::protocol_failed("MCP process output exceeds the single-frame byte limit"));
            }
            self.pending.extend_from_slice(&bytes[..content]);
            self.stdout.consume(consumed);
            if newline.is_some() {
                let mut line = std::mem::take(&mut self.pending);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return String::from_utf8(line).map_err(|_| {
                    McpOwnerError::protocol_failed("MCP process output is not UTF-8")
                });
            }
            tokio::task::yield_now().await;
        }
    }

    pub(super) async fn close(&mut self) -> Result<(), McpOwnerError> {
        self.stdin.lock().await.take();
        // Give cooperative servers a small EOF grace. Direct-child exit alone
        // is not proof: shutdown always seals/reaps the platform-owned tree.
        let _ = timeout(EOF_GRACE, self.process.child_mut().wait()).await;
        let cleanup = self.process.shutdown().await.map_err(|_| {
            McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP stdio process-tree cleanup is unproven",
            )
        });
        if let Some(mut task) = self.stderr_task.take()
            && timeout(EOF_GRACE, &mut task).await.is_err()
        {
            task.abort();
        }
        cleanup
    }

    async fn closed_stdout_error(&mut self) -> McpOwnerError {
        let mut exited = self
            .process
            .child_mut()
            .try_wait()
            .ok()
            .flatten()
            .is_some();
        if !exited {
            tokio::task::yield_now().await;
            exited = self
                .process
                .child_mut()
                .try_wait()
                .ok()
                .flatten()
                .is_some();
        }
        if exited
            && let Some(mut task) = self.stderr_task.take()
            && timeout(EOF_GRACE, &mut task).await.is_err()
        {
            task.abort();
        }
        let failure = self
            .stderr_diagnostics
            .lock()
            .ok()
            .and_then(|diagnostics| diagnostics.failure_kind());
        match failure {
            Some(StdioFailureKind::PackageNotFound) => McpOwnerError::new(
                "MCP_PACKAGE_NOT_FOUND",
                "MCP package runner could not resolve the configured package",
            ),
            Some(StdioFailureKind::Network) => McpOwnerError::new(
                "MCP_PACKAGE_DOWNLOAD_FAILED",
                "MCP process could not download or reach a required dependency",
            ),
            Some(StdioFailureKind::Permission) => McpOwnerError::new(
                "MCP_COMMAND_PERMISSION_DENIED",
                "MCP process exited after a permission failure",
            ),
            Some(StdioFailureKind::MissingDependency) => McpOwnerError::new(
                "MCP_PROCESS_DEPENDENCY_MISSING",
                "MCP process exited because a required local dependency is missing",
            ),
            Some(StdioFailureKind::Configuration) => McpOwnerError::new(
                "MCP_PROCESS_CONFIGURATION_REQUIRED",
                "MCP process exited because required runtime configuration is missing",
            ),
            None if exited => McpOwnerError::new(
                "MCP_PROCESS_EXITED",
                "MCP process exited before completing the protocol handshake",
            ),
            None => McpOwnerError::protocol_failed(
                "MCP process closed stdout before a complete correlated response",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StdioFailureKind {
    PackageNotFound,
    Network,
    Permission,
    MissingDependency,
    Configuration,
}

#[derive(Default)]
struct StdioDiagnostics {
    classified_bytes: usize,
    package_not_found: bool,
    network: bool,
    permission: bool,
    missing_dependency: bool,
    configuration: bool,
}

impl StdioDiagnostics {
    fn observe(&mut self, bytes: &[u8]) {
        // Only retain classifications. Raw stderr may contain URLs, tokens,
        // kube contexts or other credentials and must never enter API errors
        // or logs. Classification work is capped even if a child floods its
        // diagnostic stream; the reader keeps draining after the cap.
        let remaining = MAX_STDERR_DIAGNOSTIC_BYTES.saturating_sub(self.classified_bytes);
        if remaining == 0 {
            return;
        }
        let bytes = &bytes[..bytes.len().min(remaining)];
        self.classified_bytes = self.classified_bytes.saturating_add(bytes.len());
        let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
        self.package_not_found |= contains_any(
            &text,
            &[
                "e404",
                "404 not found",
                "no matching version",
                "could not find a version",
                "package not found",
            ],
        );
        self.network |= contains_any(
            &text,
            &[
                "econnrefused",
                "econnreset",
                "enotfound",
                "etimedout",
                "network error",
                "proxy error",
                "certificate error",
                "unable to get local issuer",
                "failed to download",
                "failed to fetch",
                "tls error",
                "ssl error",
            ],
        );
        self.permission |= contains_any(
            &text,
            &["eacces", "eperm", "permission denied", "access is denied"],
        );
        self.missing_dependency |= contains_any(
            &text,
            &[
                "not recognized as an internal or external command",
                "command not found",
                "no such file or directory",
                "modulenotfounderror",
                "cannot find module",
            ],
        );
        self.configuration |= contains_any(
            &text,
            &[
                "missing required configuration",
                "missing required environment",
                "api key is required",
                "no kubeconfig",
                "kubeconfig not found",
                "credentials not found",
                "login required",
                "authentication required",
            ],
        );
    }

    fn failure_kind(&self) -> Option<StdioFailureKind> {
        if self.package_not_found {
            Some(StdioFailureKind::PackageNotFound)
        } else if self.network {
            Some(StdioFailureKind::Network)
        } else if self.permission {
            Some(StdioFailureKind::Permission)
        } else if self.missing_dependency {
            Some(StdioFailureKind::MissingDependency)
        } else if self.configuration {
            Some(StdioFailureKind::Configuration)
        } else {
            None
        }
    }
}

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

async fn drain_stderr(
    mut stderr: ChildStderr,
    diagnostics: Arc<StdMutex<StdioDiagnostics>>,
) {
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                if let Ok(mut diagnostics) = diagnostics.lock() {
                    diagnostics.observe(&buffer[..read]);
                }
            }
        }
    }
}

pub(super) fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, McpOwnerError> {
    let mut frame = serde_json::to_vec(message)
        .map_err(|_| McpOwnerError::protocol_failed("MCP process request could not be encoded"))?;
    if frame.len() > MAX_RESPONSE_BYTES {
        return Err(McpOwnerError::protocol_failed(
            "MCP process request exceeds the frame limit",
        ));
    }
    frame.push(b'\n');
    Ok(frame)
}

pub(super) async fn write_frame(writer: Writer, frame: Vec<u8>) -> Result<(), McpOwnerError> {
    let mut guard = writer.lock().await;
    // Take the pipe until the entire frame is flushed. Cancellation or I/O
    // failure drops it and leaves None, so a later notification or server reply
    // can never be appended to an unfinished JSON-RPC frame.
    let mut stdin = guard
        .take()
        .ok_or_else(|| McpOwnerError::connection_failed("MCP process input is closed"))?;
    stdin
        .write_all(&frame)
        .await
        .map_err(|_| McpOwnerError::connection_failed("MCP process request write failed"))?;
    stdin
        .flush()
        .await
        .map_err(|_| McpOwnerError::connection_failed("MCP process input flush failed"))?;
    *guard = Some(stdin);
    Ok(())
}

pub(super) async fn read_response(
    session: &mut McpSession,
    expected: u64,
) -> Result<JsonRpcResponse, McpOwnerError> {
    loop {
        let line = session
            .stdio
            .as_mut()
            .ok_or_else(|| McpOwnerError::protocol_failed("MCP process transport is unavailable"))?
            .next()
            .await?;
        // Strict legacy JSON-RPC framing: stdout is protocol, stderr is not.
        // No skipping malformed replies or silently replaying after EOF.
        if let Some(response) = stream::process_message(&line, expected, session).await? {
            return Ok(response);
        }
    }
}

fn validate_config(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<(), McpOwnerError> {
    if command.is_empty()
        || command.contains('\0')
        || args.len() > 256
        || env.len() > 128
        || args.iter().any(|value| value.contains('\0'))
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP stdio command or argument list is malformed or oversized",
        ));
    }
    let mut keys = BTreeSet::new();
    let mut bytes = command.len() + args.iter().map(String::len).sum::<usize>();
    for (name, value) in env {
        let key = if cfg!(windows) {
            name.to_ascii_uppercase()
        } else {
            name.clone()
        };
        if name.is_empty()
            || name.contains('=')
            || name.chars().any(char::is_control)
            || value.contains('\0')
            || !keys.insert(key)
        {
            return Err(McpOwnerError::invalid_binding(
                "MCP stdio environment is malformed or has duplicate keys",
            ));
        }
        bytes = bytes.saturating_add(name.len()).saturating_add(value.len());
    }
    if bytes > 256 * 1024 {
        return Err(McpOwnerError::invalid_binding(
            "MCP stdio launch configuration exceeds its byte limit",
        ));
    }
    Ok(())
}

fn runtime_resolution_error(cause: &nomifun_runtime::ToolError) -> McpOwnerError {
    match cause {
        nomifun_runtime::ToolError::Shared(cause) => runtime_resolution_error(cause),
        cause => McpOwnerError::new("MCP_RUNTIME_UNAVAILABLE",
            nomi_redact::redact_secrets_owned(format!("MCP command environment could not be prepared: {cause}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_write_closes_the_pipe_before_any_later_frame_can_append() {
        use std::future::Future;
        let mut builder = ChildProcessBuilder::new("/bin/sh");
        builder.args(["-c", "sleep 60"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut process = builder.spawn_managed().unwrap();
        let writer = Arc::new(Mutex::new(process.child_mut().stdin.take()));
        let mut writing = Box::pin(write_frame(writer.clone(), vec![b'x'; 700 * 1024]));
        std::future::poll_fn(|cx| {
            assert!(writing.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        }).await;
        drop(writing);
        assert!(writer.lock().await.is_none());
        assert_eq!(write_frame(writer, b"another frame\n".to_vec()).await.unwrap_err().code(), "MCP_CONNECTION_FAILED");
        timeout(Duration::from_secs(5), process.shutdown()).await.unwrap().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn late_spawn_after_cancellation_or_deadline_retains_exact_platform_cleanup() {
        for expired in [false, true] {
            let mut builder = ChildProcessBuilder::new("/bin/sh");
            builder.args(["-c", "sleep 60"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let process = builder.spawn_managed().unwrap();
            let proof = process.cleanup_receipt().expect("managed spawn registered the exact tree owner");
            let control = McpInvocationControl::default();
            if !expired { control.cancellation.cancel(); }
            let deadline = if expired { Instant::now() } else { Instant::now() + Duration::from_secs(60) };
            let error = match accept_launched_process(process, &control.cancellation, deadline) {
                Ok(_) => panic!("late launched process escaped its interrupted owner"),
                Err(error) => error,
            };
            let outcome: super::super::McpInvocationOutcome<()> = super::super::startup_failure(error, &control);
            assert!(outcome.execution.local_process_started);
            assert_eq!(outcome.cleanup.local, McpCleanupStatus::Unproven);
            timeout(Duration::from_secs(5), proof.wait()).await.expect("platform reaper retained the late tree").unwrap();
            // Completion here is an independently awaited platform proof. The
            // earlier returned facts correctly do not retroactively claim it.
            assert_eq!(outcome.cleanup.local, McpCleanupStatus::Unproven);
        }
    }

    #[tokio::test]
    async fn runtime_resolution_does_not_replace_explicit_user_stdio_paths() {
        let command = "/user/chosen/bun";
        let args = vec!["server.js".to_owned()];
        let launch =
            nomifun_runtime::prepare_process_launch(command, &args, Path::new("."), &HashMap::new())
                .await
                .unwrap();
        assert_eq!(launch.program, command);
        assert_eq!(launch.args, args);
    }

    #[tokio::test]
    async fn configured_child_path_bun_precedes_runtime_supply_and_preserves_bunx() {
        let before = nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::Bun);
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join(if cfg!(windows) { "bun.exe" } else { "bun" });
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = HashMap::from([("PATH".to_owned(), root.path().to_string_lossy().into_owned())]);
        let args = vec!["server.js".into()];
        for (command, expected_args) in [("bun", args.clone()), ("bunx", vec!["x".into(), "server.js".into()])] {
            let launch = nomifun_runtime::prepare_process_launch(
                command, &args, root.path(), &env,
            ).await.unwrap();
            assert_eq!(Path::new(&launch.program), program);
            assert_eq!(launch.args, expected_args);
        }
        assert_eq!(nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::Bun), before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_native_argv_preserves_empty_unicode_quotes_and_metacharacters() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join(" MCP fixture 中文\n ");
        let marker = root.path().join("must-not-exist");
        std::fs::write(&program, r#"#!/bin/sh
[ "$#" = 4 ] || exit 91
[ -z "$1" ] || exit 92
[ "$2" = '中文 space' ] || exit 93
[ "$3" = "quote'\"" ] || exit 94
[ "$4" = "$EXPECTED_LITERAL" ] || exit 95
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"argv-fixture","version":"1"}}}' ;;
    *'"id":2'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}'; exit 0 ;;
  esac
done
"#).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let literal = format!("$(touch '{}'); & | >", marker.display());
        let environment = HashMap::from([("EXPECTED_LITERAL".into(), literal.clone())]);
        let catalog = discover(program.to_str().unwrap(), &["".into(), "中文 space".into(), "quote'\"".into(), literal],
            &environment, &McpSetupPolicy::uniform(Duration::from_secs(5))).await.unwrap();
        assert_eq!(catalog["tools"], serde_json::json!([]));
        assert!(!marker.exists(), "native argv must never become shell code");
    }

    #[test]
    fn native_executable_names_preserve_literal_whitespace_and_line_breaks() {
        for command in [" ", "  executable  ", "line\nbreak", "path/ trailing "] {
            validate_config(command, &[], &HashMap::new()).unwrap();
        }
        for command in ["", "name\0suffix"] {
            assert!(validate_config(command, &[], &HashMap::new()).is_err());
        }
    }

    #[test]
    fn runtime_preparation_failure_preserves_its_real_reason() {
        let missing = nomifun_runtime::ToolError::Shared(Arc::new(nomifun_runtime::ToolError::NotPrepared));
        let error = runtime_resolution_error(&missing);
        assert_eq!(error.code(), "MCP_RUNTIME_UNAVAILABLE");
        assert!(error.message().contains("could not be prepared"));
    }

    #[test]
    fn stderr_is_reduced_to_non_secret_failure_categories() {
        let mut diagnostics = StdioDiagnostics::default();
        diagnostics.observe(b"npm ERR! code ECONNRESET https://user:secret@example.test/pkg?token=x");

        assert_eq!(diagnostics.failure_kind(), Some(StdioFailureKind::Network));
        assert!(std::mem::size_of_val(&diagnostics) <= 16);
    }

    #[test]
    fn stderr_failure_category_priority_is_stable() {
        let mut diagnostics = StdioDiagnostics::default();
        diagnostics.observe(b"network error; npm ERR! E404 package not found");

        assert_eq!(
            diagnostics.failure_kind(),
            Some(StdioFailureKind::PackageNotFound)
        );
    }

    #[test]
    fn stderr_classification_work_is_bounded_while_the_pipe_remains_drainable() {
        let mut diagnostics = StdioDiagnostics::default();
        diagnostics.observe(&vec![b'x'; MAX_STDERR_DIAGNOSTIC_BYTES * 2]);
        diagnostics.observe(b"npm ERR! code ECONNRESET");

        assert_eq!(diagnostics.classified_bytes, MAX_STDERR_DIAGNOSTIC_BYTES);
        assert_eq!(diagnostics.failure_kind(), None);
    }

}
