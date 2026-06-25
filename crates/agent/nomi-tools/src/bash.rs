use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_config::shell::shell_command_builder;
use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::output_truncation::{truncate_middle, TruncationBudget};
use crate::Tool;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Per-stream byte budget for Bash output before head/tail elision. Matches
/// `Tool::max_result_size()` so the engine-level fallback rarely fires.
const BASH_OUTPUT_MAX_BYTES: usize = 50_000;

pub struct BashTool {
    cwd: PathBuf,
    /// When set, commands run in a long-lived shell session so cwd/env persist
    /// across calls (Unix-only, dark-launch). `None` → stateless one-shot.
    #[cfg(unix)]
    persistent: Option<std::sync::Arc<crate::persistent_shell::PersistentShell>>,
    /// When set (macOS only), commands run under a Seatbelt write-containment
    /// sandbox allowing writes only to these roots (+ temp/devices). `None` = off.
    sandbox_roots: Option<Vec<PathBuf>>,
}

impl BashTool {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            #[cfg(unix)]
            persistent: None,
            sandbox_roots: None,
        }
    }

    /// Construct a `Bash` tool backed by a persistent shell session. cwd/env
    /// mutations persist across calls. Unix-only.
    #[cfg(unix)]
    pub fn with_persistent_shell(
        cwd: PathBuf,
        shell: std::sync::Arc<crate::persistent_shell::PersistentShell>,
    ) -> Self {
        Self {
            cwd,
            persistent: Some(shell),
            sandbox_roots: None,
        }
    }

    /// Run commands under a macOS Seatbelt write-containment sandbox, allowing
    /// writes only to `roots` (plus temp dirs and the standard devices). No-op
    /// on non-macOS. (§3.6 OS sandbox)
    pub fn with_sandbox(mut self, roots: Option<Vec<PathBuf>>) -> Self {
        self.sandbox_roots = roots;
        self
    }

    /// Run `command` in the persistent shell session and format the result with
    /// the same envelope as the one-shot path. stdout/stderr are PTY-interleaved
    /// (a single stream), so they are reported together.
    #[cfg(unix)]
    async fn execute_persistent(
        &self,
        shell: &crate::persistent_shell::PersistentShell,
        command: &str,
        timeout_ms: u64,
    ) -> ToolResult {
        match shell
            .run(command, Duration::from_millis(timeout_ms))
            .await
        {
            Ok(outcome) if outcome.timed_out => ToolResult {
                content: format!(
                    "Command timed out after {}ms (the shell was interrupted).\nPartial output:\n{}",
                    timeout_ms,
                    truncate_middle(&outcome.output, TruncationBudget::Bytes(BASH_OUTPUT_MAX_BYTES)),
                ),
                is_error: true,
                images: Vec::new(),
            },
            Ok(outcome) => {
                let output =
                    truncate_middle(&outcome.output, TruncationBudget::Bytes(BASH_OUTPUT_MAX_BYTES));
                ToolResult {
                    content: format!("Exit code: {}\nOUTPUT:\n{}", outcome.exit_code, output),
                    is_error: outcome.exit_code != 0,
                    images: Vec::new(),
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to run command in persistent shell: {e}"),
                is_error: true,
                images: Vec::new(),
            },
        }
    }

    /// Run `command` under a macOS Seatbelt write-containment sandbox
    /// (`sandbox-exec`), allowing writes only to `roots` (+ temp/devices).
    #[cfg(target_os = "macos")]
    async fn execute_sandboxed(&self, roots: &[PathBuf], command: &str, timeout_ms: u64) -> ToolResult {
        let profile = crate::sandbox::write_sandbox_profile(roots);
        let timeout = Duration::from_millis(timeout_ms);
        let cwd = self.cwd.clone();
        let result = tokio::time::timeout(timeout, async {
            let mut cmd = tokio::process::Command::new("/usr/bin/sandbox-exec");
            cmd.arg("-p")
                .arg(&profile)
                .arg("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&cwd);
            // Strip dynamic-linker injection vars from the sandboxed subprocess.
            crate::sandbox::harden_env(&mut cmd);
            cmd.output().await
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);
                let stdout = truncate_middle(&stdout, TruncationBudget::Bytes(BASH_OUTPUT_MAX_BYTES));
                let stderr =
                    truncate_middle(&stderr, TruncationBudget::Bytes(BASH_OUTPUT_MAX_BYTES / 2));
                ToolResult {
                    content: format!(
                        "Exit code: {} [sandboxed]\nSTDOUT:\n{}\nSTDERR:\n{}",
                        exit_code, stdout, stderr
                    ),
                    is_error: exit_code != 0,
                    images: Vec::new(),
                }
            }
            Ok(Err(e)) => ToolResult {
                content: format!("Failed to execute sandboxed command: {}", e),
                is_error: true,
                images: Vec::new(),
            },
            Err(_) => ToolResult {
                content: format!("Command timed out after {}ms", timeout_ms),
                is_error: true,
                images: Vec::new(),
            },
        }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Executes a shell command and returns its output.\n\n\
         IMPORTANT: Do NOT use Bash when a dedicated tool is available:\n\
         - File search: use Glob (not find or ls)\n\
         - Content search: use Grep (not grep or rg)\n\
         - Read files: use Read (not cat, head, or tail)\n\
         - Edit files: use Edit (not sed or awk)\n\
         - Write files: use Write (not echo or cat with heredoc)\n\n\
         # Instructions\n\
         - Use absolute paths to avoid working directory confusion.\n\
         - When issuing multiple independent commands, make parallel tool calls \
         instead of chaining them. Use `&&` only when commands depend on each other.\n\
         - You may specify an optional timeout in milliseconds (default 120000, max 600000).\n\n\
         # Git safety\n\
         - Never force push, reset --hard, or use --no-verify unless explicitly asked.\n\
         - Prefer creating new commits over amending existing ones."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 120000, max 600000)"
                }
            },
            "required": ["command"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(command) = input["command"].as_str() else {
            return ToolResult {
                content: "Missing required parameter: command".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        };

        tracing::debug!(cwd = %self.cwd.display(), command = %command, "BashTool executing");

        let timeout_ms = input["timeout"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        // macOS Seatbelt write-containment sandbox (opt-in): takes precedence so
        // arbitrary subprocesses are confined. Falls through if unsupported.
        #[cfg(target_os = "macos")]
        if let Some(roots) = &self.sandbox_roots {
            if crate::sandbox::is_supported() {
                return self.execute_sandboxed(roots, command, timeout_ms).await;
            }
        }

        // Persistent-shell path (Unix, dark-launch): cwd/env persist across calls.
        #[cfg(unix)]
        if let Some(shell) = &self.persistent {
            return self.execute_persistent(shell, command, timeout_ms).await;
        }

        let timeout = Duration::from_millis(timeout_ms);

        let cwd = self.cwd.clone();
        let result = tokio::time::timeout(timeout, async {
            shell_command_builder(command)
                .current_dir(&cwd)
                .output()
                .await
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                // Bound each stream independently so a noisy stdout can't crowd
                // out stderr (and vice versa); head/tail elision keeps both ends.
                let stdout = truncate_middle(&stdout, TruncationBudget::Bytes(BASH_OUTPUT_MAX_BYTES));
                let stderr =
                    truncate_middle(&stderr, TruncationBudget::Bytes(BASH_OUTPUT_MAX_BYTES / 2));

                let content = format!(
                    "Exit code: {}\nSTDOUT:\n{}\nSTDERR:\n{}",
                    exit_code, stdout, stderr
                );

                ToolResult {
                    content,
                    is_error: exit_code != 0,
                    images: Vec::new(),
                }
            }
            Ok(Err(e)) => ToolResult {
                content: format!("Failed to execute command: {}", e),
                is_error: true,
                images: Vec::new(),
            },
            Err(_) => ToolResult {
                content: format!("Command timed out after {}ms", timeout_ms),
                is_error: true,
                images: Vec::new(),
            },
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    fn describe(&self, input: &Value) -> String {
        let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        format!("Execute: {}", crate::truncate_utf8(cmd, 80))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn execute_echo_returns_stdout() {
        let tool = BashTool::new(std::env::temp_dir());
        let input = json!({"command": "echo hello_bash"});
        let result = tool.execute(input).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("hello_bash"));
    }

    #[tokio::test]
    async fn execute_invalid_command_returns_error() {
        let tool = BashTool::new(std::env::temp_dir());
        let input = json!({"command": "nonexistent_command_xyz_123"});
        let result = tool.execute(input).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn execute_respects_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cwd_proof.txt"), "proof").unwrap();
        let tool = BashTool::new(dir.path().to_path_buf());
        let cmd = if cfg!(windows) {
            "type cwd_proof.txt"
        } else {
            "cat cwd_proof.txt"
        };
        let input = json!({"command": cmd});
        let result = tool.execute(input).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(
            result.content.contains("proof"),
            "BashTool should execute in injected cwd, got: {}",
            result.content
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn persistent_shell_path_persists_cwd_across_calls() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("inner");
        std::fs::create_dir(&sub).unwrap();
        let shell = Arc::new(crate::persistent_shell::PersistentShell::new(
            dir.path().to_string_lossy().into_owned(),
        ));
        let tool = BashTool::with_persistent_shell(dir.path().to_path_buf(), shell);

        // First call changes directory; second must observe it (one-shot Bash
        // would not — this is the persistent-shell guarantee).
        let cd = tool.execute(json!({"command": format!("cd {}", sub.display())})).await;
        assert!(!cd.is_error, "cd failed: {}", cd.content);
        let pwd = tool.execute(json!({"command": "pwd"})).await;
        assert!(
            pwd.content.contains("inner"),
            "cwd must persist across Bash calls in persistent mode, got: {}",
            pwd.content
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sandbox_blocks_writes_outside_the_workspace_root() {
        if !crate::sandbox::is_supported() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let canon = root.path().canonicalize().unwrap();
        let tool = BashTool::new(canon.clone()).with_sandbox(Some(vec![canon.clone()]));

        // Write inside the workspace → allowed.
        let inside = canon.join("inside.txt");
        let ok = tool
            .execute(json!({ "command": format!("echo hi > {}", inside.display()) }))
            .await;
        assert!(!ok.is_error, "in-root write should succeed: {}", ok.content);
        assert!(inside.exists());

        // Write to $HOME (outside) → blocked by the sandbox (non-zero exit).
        let home = std::env::var("HOME").unwrap();
        let outside = std::path::Path::new(&home).join(".nomi_bash_sandbox_escape.txt");
        let _ = std::fs::remove_file(&outside);
        let denied = tool
            .execute(json!({ "command": format!("echo hi > {}", outside.display()) }))
            .await;
        let escaped = outside.exists();
        let _ = std::fs::remove_file(&outside);
        assert!(denied.is_error, "out-of-root write should report failure");
        assert!(!escaped, "out-of-root write must be blocked by the sandbox");
    }
}
