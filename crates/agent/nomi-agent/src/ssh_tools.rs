//! The SSH remote tool family. Each tool takes over the provider-visible name
//! of a native local tool (`Bash`/`Read`/`Edit`/`Write`/`Grep`/`Glob`) so the
//! model operates the remote host through its ordinary vocabulary — no `ssh_*`
//! second family, no prompt changes to teach the model which machine to target.
//! The bootstrap selects this family instead of the local one when a session is
//! bound to an SSH host (see `AgentBootstrap::ssh_session`).
//!
//! Every tool delegates to `SshBackend`; none touches the local filesystem or
//! process runtime. The `ssh_tool_contract` test enforces that.
use std::sync::Arc;

use async_trait::async_trait;
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};
use serde_json::{Value, json};

use crate::ssh_backend::SshBackend;

/// Default per-command timeout (30s) — matches the model's expectation for a
/// bounded shell command; long-running work should be backgrounded remotely.
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 30_000;

fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, ToolResult> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolResult::error(format!("Missing required parameter: {key}")))
}

// ── Bash ──────────────────────────────────────────────────────────────────

pub struct SshBashTool {
    backend: Arc<dyn SshBackend>,
}

impl SshBashTool {
    pub fn new(backend: Arc<dyn SshBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for SshBashTool {
    fn name(&self) -> &str {
        "Bash"
    }
    fn description(&self) -> &str {
        "Run a shell command on the remote host over SSH. State (cwd, environment, \
         activated virtualenvs) persists across calls within the session."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to run on the remote host." }
            },
            "required": ["command"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // Shares one stateful remote shell; commands must serialize.
        false
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let command = match str_arg(&input, "command") {
            Ok(c) => c,
            Err(e) => return e,
        };
        match self
            .backend
            .run_command(command, DEFAULT_COMMAND_TIMEOUT_MS)
            .await
        {
            Ok(out) => {
                let mut content = out.stdout;
                if out.timed_out {
                    content.push_str("\n[command timed out]");
                }
                // A nonzero exit is reported in-band; not a tool error.
                if out.exit_code != 0 {
                    content.push_str(&format!("\n[exit code: {}]", out.exit_code));
                }
                ToolResult::text(content)
            }
            Err(e) => ToolResult::error(format!("remote command failed: {e}")),
        }
    }
}

// ── Read ──────────────────────────────────────────────────────────────────

pub struct SshReadTool {
    backend: Arc<dyn SshBackend>,
}

impl SshReadTool {
    pub fn new(backend: Arc<dyn SshBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for SshReadTool {
    fn name(&self) -> &str {
        "Read"
    }
    fn description(&self) -> &str {
        "Read a file from the remote host over SFTP."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Absolute path on the remote host." }
            },
            "required": ["file_path"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let path = match str_arg(&input, "file_path") {
            Ok(p) => p,
            Err(e) => return e,
        };
        match self.backend.read_file(path).await {
            Ok(bytes) => ToolResult::text(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) => ToolResult::error(format!("remote read failed: {e}")),
        }
    }
}

// ── Write ─────────────────────────────────────────────────────────────────

pub struct SshWriteTool {
    backend: Arc<dyn SshBackend>,
}

impl SshWriteTool {
    pub fn new(backend: Arc<dyn SshBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for SshWriteTool {
    fn name(&self) -> &str {
        "Write"
    }
    fn description(&self) -> &str {
        "Write a file on the remote host over SFTP (atomic: temp + rename)."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Absolute path on the remote host." },
                "content": { "type": "string", "description": "File contents." }
            },
            "required": ["file_path", "content"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let path = match str_arg(&input, "file_path") {
            Ok(p) => p,
            Err(e) => return e,
        };
        let content = match str_arg(&input, "content") {
            Ok(c) => c,
            Err(e) => return e,
        };
        match self.backend.write_file(path, content.as_bytes().to_vec()).await {
            Ok(()) => ToolResult::text(format!("Wrote {} bytes to {path}", content.len())),
            Err(e) => ToolResult::error(format!("remote write failed: {e}")),
        }
    }
}

// ── Edit ──────────────────────────────────────────────────────────────────

pub struct SshEditTool {
    backend: Arc<dyn SshBackend>,
}

impl SshEditTool {
    pub fn new(backend: Arc<dyn SshBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for SshEditTool {
    fn name(&self) -> &str {
        "Edit"
    }
    fn description(&self) -> &str {
        "Replace an exact string in a remote file (read over SFTP, substitute, \
         atomic write-back). `old_string` must be unique in the file."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Absolute path on the remote host." },
                "old_string": { "type": "string", "description": "Exact text to replace (must be unique)." },
                "new_string": { "type": "string", "description": "Replacement text." }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let path = match str_arg(&input, "file_path") {
            Ok(p) => p,
            Err(e) => return e,
        };
        let old = match str_arg(&input, "old_string") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let new = match str_arg(&input, "new_string") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let current = match self.backend.read_file(path).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => return ToolResult::error(format!("remote read failed: {e}")),
        };
        let matches = current.matches(old).count();
        if matches == 0 {
            return ToolResult::error(format!("old_string not found in {path}"));
        }
        if matches > 1 {
            return ToolResult::error(format!(
                "old_string is not unique in {path} ({matches} occurrences); include more context"
            ));
        }
        let updated = current.replacen(old, new, 1);
        match self.backend.write_file(path, updated.into_bytes()).await {
            Ok(()) => ToolResult::text(format!("Edited {path}")),
            Err(e) => ToolResult::error(format!("remote write failed: {e}")),
        }
    }
}

// ── Grep ──────────────────────────────────────────────────────────────────

pub struct SshGrepTool {
    backend: Arc<dyn SshBackend>,
}

impl SshGrepTool {
    pub fn new(backend: Arc<dyn SshBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for SshGrepTool {
    fn name(&self) -> &str {
        "Grep"
    }
    fn description(&self) -> &str {
        "Search remote files for a pattern (ripgrep if available, else grep)."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex to search for." },
                "path": { "type": "string", "description": "Remote directory to search (default: cwd)." }
            },
            "required": ["pattern"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let pattern = match str_arg(&input, "pattern") {
            Ok(p) => p,
            Err(e) => return e,
        };
        let path = input.get("path").and_then(Value::as_str).unwrap_or(".");
        match self.backend.grep(pattern, path).await {
            Ok(out) => ToolResult::text(out),
            Err(e) => ToolResult::error(format!("remote grep failed: {e}")),
        }
    }
}

// ── Glob ──────────────────────────────────────────────────────────────────

pub struct SshGlobTool {
    backend: Arc<dyn SshBackend>,
}

impl SshGlobTool {
    pub fn new(backend: Arc<dyn SshBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for SshGlobTool {
    fn name(&self) -> &str {
        "Glob"
    }
    fn description(&self) -> &str {
        "List remote files matching a glob pattern."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern, e.g. src/**/*.rs" }
            },
            "required": ["pattern"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let pattern = match str_arg(&input, "pattern") {
            Ok(p) => p,
            Err(e) => return e,
        };
        match self.backend.list_files(pattern).await {
            Ok(entries) => ToolResult::text(entries.join("\n")),
            Err(e) => ToolResult::error(format!("remote glob failed: {e}")),
        }
    }
}

/// Build the full remote tool family bound to `backend`. The bootstrap registers
/// these instead of the local `Read`/`Write`/`Edit`/`Bash`/`Grep`/`Glob` when a
/// session is bound to an SSH host.
pub fn remote_tool_family(backend: Arc<dyn SshBackend>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(SshReadTool::new(backend.clone())),
        Box::new(SshWriteTool::new(backend.clone())),
        Box::new(SshEditTool::new(backend.clone())),
        Box::new(SshBashTool::new(backend.clone())),
        Box::new(SshGrepTool::new(backend.clone())),
        Box::new(SshGlobTool::new(backend)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockBackend {
        last_command: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl SshBackend for MockBackend {
        async fn run_command(
            &self,
            command: &str,
            _timeout_ms: u64,
        ) -> Result<crate::ssh_backend::RemoteCommandOutput, String> {
            *self.last_command.lock().unwrap() = Some(command.to_string());
            Ok(crate::ssh_backend::RemoteCommandOutput {
                stdout: format!("ran: {command}"),
                exit_code: 0,
                timed_out: false,
            })
        }
        async fn read_file(&self, _path: &str) -> Result<Vec<u8>, String> {
            Ok(b"hello\nworld\n".to_vec())
        }
        async fn write_file(&self, _path: &str, _bytes: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
        async fn grep(&self, _pattern: &str, _path: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn list_files(&self, _glob: &str) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        async fn stat(
            &self,
            _path: &str,
        ) -> Result<crate::ssh_backend::RemoteFileStat, String> {
            Ok(crate::ssh_backend::RemoteFileStat { size: 0, is_dir: false })
        }
    }

    fn mock() -> Arc<MockBackend> {
        Arc::new(MockBackend {
            last_command: std::sync::Mutex::new(None),
        })
    }

    #[test]
    fn remote_tools_take_over_native_names() {
        let b = mock();
        assert_eq!(SshBashTool::new(b.clone()).name(), "Bash");
        assert_eq!(SshReadTool::new(b.clone()).name(), "Read");
        assert_eq!(SshWriteTool::new(b.clone()).name(), "Write");
        assert_eq!(SshEditTool::new(b.clone()).name(), "Edit");
        assert_eq!(SshGrepTool::new(b.clone()).name(), "Grep");
        assert_eq!(SshGlobTool::new(b).name(), "Glob");
    }

    #[tokio::test]
    async fn bash_routes_to_backend_and_is_exec() {
        let b = mock();
        let tool = SshBashTool::new(b.clone());
        assert!(matches!(tool.category(), ToolCategory::Exec));
        assert!(!tool.is_concurrency_safe(&json!({})));
        let res = tool.execute(json!({"command": "echo hi"})).await;
        assert!(!res.is_error, "got: {}", res.content);
        assert!(res.content.contains("ran: echo hi"));
        assert_eq!(b.last_command.lock().unwrap().as_deref(), Some("echo hi"));
    }

    #[tokio::test]
    async fn read_is_info_and_returns_content() {
        let tool = SshReadTool::new(mock());
        assert!(matches!(tool.category(), ToolCategory::Info));
        let res = tool.execute(json!({"file_path": "/etc/hostname"})).await;
        assert!(res.content.contains("hello"));
    }

    #[tokio::test]
    async fn edit_requires_unique_old_string() {
        // read_file returns "hello\nworld\n"; "l" appears many times.
        let tool = SshEditTool::new(mock());
        let res = tool
            .execute(json!({"file_path": "/f", "old_string": "l", "new_string": "L"}))
            .await;
        assert!(res.is_error, "non-unique old_string must error");
        assert!(res.content.contains("not unique"));
    }
}
