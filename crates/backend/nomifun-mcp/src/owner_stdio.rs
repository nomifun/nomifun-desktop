//! Local MCP byte transport. Protocol traffic does not grant platform tools;
//! process placement, tree cleanup and inherited environment remain host-owned.
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess, resolve_command_in};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout, timeout_at};

use super::{JsonRpcResponse, MAX_RESPONSE_BYTES, McpOwnerError, McpSession, stream};

pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const EOF_GRACE: Duration = Duration::from_millis(250);
const MAX_TRANSACTION_BYTES: usize = 32 * 1024 * 1024;
const MAX_FRAMES: usize = 4096;
const MAX_STDERR_DIAGNOSTIC_BYTES: usize = 64 * 1024;
pub(super) type Writer = Arc<Mutex<Option<ChildStdin>>>;

/// Settings discovery uses the same process owner and full bounded catalog.
/// Its timeout cannot skip cleanup or publish a catalog with unproven cleanup.
pub(crate) async fn discover(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    budget: Duration,
) -> Result<serde_json::Value, McpOwnerError> {
    let deadline = Instant::now() + budget;
    let transport = StdioTransport::launch(command, args, env, deadline).await?;
    let mut session = McpSession::from_stdio(transport);
    let outcome = timeout_at(deadline, async {
        session.initialize().await?;
        let (_, tools) = session.read_catalog(None).await?;
        Ok(serde_json::json!({"tools": tools}))
    })
    .await
    .unwrap_or_else(|_| {
        Err(McpOwnerError::new(
            "MCP_TIMEOUT",
            "MCP catalog discovery timed out",
        ))
    });
    timeout(CLEANUP_TIMEOUT, session.close())
        .await
        .map_err(|_| {
            McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP catalog process cleanup timed out; cleanup remains unproven",
            )
        })??;
    outcome
}

pub(super) struct StdioTransport {
    // Drop transfers unfinished cleanup to the platform relay, not a raw PID
    // kill. Only successful close() is evidence returned to the MCP owner.
    process: ManagedChildProcess,
    stdin: Writer,
    stdout: BufReader<ChildStdout>,
    pending: Vec<u8>,
    received: usize,
    frames: usize,
    stderr_diagnostics: Arc<StdMutex<StdioDiagnostics>>,
    stderr_task: Option<JoinHandle<()>>,
}

impl StdioTransport {
    pub(super) async fn launch(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        deadline: Instant,
    ) -> Result<Self, McpOwnerError> {
        validate_config(command, args, env)?;
        let command = command.to_owned();
        let args = args.to_vec();
        let configured_env = env.clone();
        // Resolution and platform spawn can block. If the waiter is cancelled
        // or times out, the task's result is dropped and ManagedChildProcess
        // still owns cleanup. A startup timeout is never a no-effect proof.
        let launch = tokio::task::spawn_blocking(move || {
            if Instant::now() >= deadline {
                return Err(startup_timeout());
            }
            let env = child_environment(&configured_env);
            let program = resolve_program(&command, &env)?;
            let mut builder = ChildProcessBuilder::new(program);
            builder
                .env_clear()
                .envs(env)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                // stderr is diagnostic-only and is drained into bounded,
                // non-secret categories below. Never merge it into stdout:
                // stdout is the MCP JSON-RPC transport.
                .stderr(Stdio::piped());
            if Instant::now() >= deadline {
                return Err(startup_timeout());
            }
            let mut process = builder.spawn_managed().map_err(|error| {
                let code = match error.kind() {
                    std::io::ErrorKind::NotFound => "MCP_COMMAND_NOT_FOUND",
                    std::io::ErrorKind::PermissionDenied => "MCP_COMMAND_PERMISSION_DENIED",
                    _ => "MCP_PROCESS_START_FAILED",
                };
                McpOwnerError::new(code, "MCP configured stdio process could not be started")
            })?;
            let stdin = process.child_mut().stdin.take().ok_or_else(|| {
                McpOwnerError::protocol_failed("MCP process stdin is unavailable")
            })?;
            let stdout = process.child_mut().stdout.take().ok_or_else(|| {
                McpOwnerError::protocol_failed("MCP process stdout is unavailable")
            })?;
            let stderr = process.child_mut().stderr.take().ok_or_else(|| {
                McpOwnerError::protocol_failed("MCP process stderr is unavailable")
            })?;
            Ok((process, stdin, stdout, stderr))
        });
        let (process, stdin, stdout, stderr) = timeout_at(deadline, launch)
            .await
            .map_err(|_| startup_timeout())?
            .map_err(|_| {
                McpOwnerError::new(
                    "MCP_PROCESS_START_FAILED",
                    "MCP process launch owner did not return",
                )
            })??;
        let stderr_diagnostics = Arc::new(StdMutex::new(StdioDiagnostics::default()));
        let stderr_task = tokio::spawn(drain_stderr(stderr, Arc::clone(&stderr_diagnostics)));
        Ok(Self {
            process,
            stdin: Arc::new(Mutex::new(Some(stdin))),
            stdout: BufReader::new(stdout),
            pending: Vec::new(),
            received: 0,
            frames: 0,
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
            self.received = self.received.saturating_add(consumed);
            if self.received > MAX_TRANSACTION_BYTES
                || content > MAX_RESPONSE_BYTES.saturating_sub(self.pending.len())
            {
                return Err(McpOwnerError::protocol_failed(
                    "MCP process output exceeds the frame or transaction byte limit",
                ));
            }
            self.pending.extend_from_slice(&bytes[..content]);
            self.stdout.consume(consumed);
            if newline.is_some() {
                self.frames += 1;
                if self.frames > MAX_FRAMES {
                    return Err(McpOwnerError::protocol_failed(
                        "MCP process output exceeds the transaction frame limit",
                    ));
                }
                let mut line = std::mem::take(&mut self.pending);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return String::from_utf8(line).map_err(|_| {
                    McpOwnerError::protocol_failed("MCP process output is not UTF-8")
                });
            }
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
    let stdin = guard
        .as_mut()
        .ok_or_else(|| McpOwnerError::connection_failed("MCP process input is closed"))?;
    stdin
        .write_all(&frame)
        .await
        .map_err(|_| McpOwnerError::connection_failed("MCP process request write failed"))?;
    stdin
        .flush()
        .await
        .map_err(|_| McpOwnerError::connection_failed("MCP process input flush failed"))
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

fn startup_timeout() -> McpOwnerError {
    McpOwnerError::new(
        "MCP_TIMEOUT",
        "MCP process startup timed out; launch or cleanup may still be pending",
    )
}

fn validate_config(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<(), McpOwnerError> {
    if command.is_empty()
        || command.trim() != command
        || command.chars().any(char::is_control)
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

fn child_environment(configured: &HashMap<String, String>) -> HashMap<OsString, OsString> {
    // No inherited API tokens, node injections or private platform variables.
    // Proxy variables come from explicit server config or the shared system-
    // proxy detector, which also suppresses stale loopback proxies. This is
    // essential for first-run npx/uvx downloads launched by a desktop process.
    #[cfg(windows)]
    const INHERIT: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
    ];
    #[cfg(not(windows))]
    const INHERIT: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LC_ALL",
        "TMPDIR",
        "TZ",
        "__CF_USER_TEXT_ENCODING",
    ];
    let mut env: HashMap<OsString, OsString> = INHERIT
        .iter()
        .filter_map(|key| std::env::var_os(*key).map(|value| (OsString::from(*key), value)))
        .collect();
    for (name, value) in
        nomifun_net::proxy::isolated_child_proxy_env(configured.keys().map(String::as_str))
    {
        env.insert(OsString::from(name), OsString::from(value));
    }
    for (name, value) in configured {
        if cfg!(windows) {
            env.retain(|key, _| !key.to_string_lossy().eq_ignore_ascii_case(name));
        }
        env.insert(OsString::from(name), OsString::from(value));
    }
    env
}

fn resolve_program(
    command: &str,
    env: &HashMap<OsString, OsString>,
) -> Result<PathBuf, McpOwnerError> {
    let path = Path::new(command);
    // Pin a supplied relative path before creating the child. Do not turn a
    // configured argv array into a shell command or infer a Session workspace.
    if path.is_absolute() || command.contains('/') || command.contains('\\') {
        return std::path::absolute(path)
            .map_err(|_| McpOwnerError::invalid_binding("MCP command path could not be resolved"));
    }
    let search = env
        .iter()
        .find(|(key, _)| {
            if cfg!(windows) {
                key.to_string_lossy().eq_ignore_ascii_case("PATH")
            } else {
                key.as_os_str() == std::ffi::OsStr::new("PATH")
            }
        })
        .map(|(_, value)| value)
        .ok_or_else(|| {
            McpOwnerError::invalid_binding(
                "MCP bare command requires an explicit or inherited PATH",
            )
        })?;
    for directory in std::env::split_paths(search) {
        let directory = std::path::absolute(directory).map_err(|_| {
            McpOwnerError::invalid_binding("MCP search directory could not be resolved")
        })?;
        if let Some(program) = resolve_command_in(command, &directory) {
            return Ok(program);
        }
    }
    Err(McpOwnerError::new(
        "MCP_COMMAND_NOT_FOUND",
        "MCP configured command was not found in the child PATH",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn isolated_child_environment_uses_shared_proxy_policy() {
        let configured = HashMap::new();
        let expected = nomifun_net::proxy::isolated_child_proxy_env(std::iter::empty());
        let actual = child_environment(&configured);

        for (name, value) in expected {
            assert_eq!(
                actual.get(&OsString::from(name)),
                Some(&OsString::from(value))
            );
        }

        let configured = HashMap::from([(
            "HTTP_PROXY".to_owned(),
            "http://configured.invalid:8080".to_owned(),
        )]);
        let actual = child_environment(&configured);
        let configured_proxy = actual.iter().find(|(name, _)| {
            name.to_string_lossy().eq_ignore_ascii_case("HTTP_PROXY")
        });
        assert_eq!(
            configured_proxy.map(|(_, value)| value),
            Some(&OsString::from("http://configured.invalid:8080"))
        );
    }
}
