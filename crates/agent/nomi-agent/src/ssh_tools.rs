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

/// Per-command timeout budget. Deliberately identical to `nomi_tools::bash`:
/// this tool occupies the provider-visible `Bash` name, so a model that learned
/// "default 120000, max 600000" from the local tool must get the same contract
/// here — otherwise every `apt-get install` / `cargo build` / migration it sends
/// with an explicit budget dies early on the remote host.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Ceiling on a single remote `Read`. SFTP hands the whole file back as one
/// `Vec<u8>` that then gets copied into a `String`, and that lands in the user's
/// desktop process — so `Read` on a log file or a core dump would park hundreds
/// of megabytes there. The local `Read` never can: it advertises a 100 KB result
/// budget and truncates. 1 MiB leaves ordinary source and config files
/// untouched while making the pathological case a refusal.
const MAX_REMOTE_READ_BYTES: u64 = 1024 * 1024;

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
         activated virtualenvs) persists across calls within the session.\n\n\
         # Instructions\n\
         - Use absolute remote paths to avoid working directory confusion.\n\
         - The remote shell is POSIX sh, not bash: no `[[ ]]`, no arrays, and no `**` globstar. \
         Use `find` for recursive listing.\n\
         - You may specify an optional timeout in milliseconds (default 120000, max 600000).\n\
         - For installs, dependency downloads, builds, migrations, or other long commands, choose a \
         generous explicit timeout. If the work can outlast the maximum, detach it \
         (`nohup <command> > /tmp/<name>.log 2>&1 &`) and poll the log file with later calls \
         instead of letting it be killed.\n\
         - Commands share one stateful remote shell, so they run one at a time in the order issued."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to run on the remote host." },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 120000, max 600000)"
                }
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
        // Same clamp as the local Bash: an absent budget takes the default, an
        // over-budget one is capped rather than silently discarded.
        let timeout_ms = input["timeout"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
        match self.backend.run_command(command, timeout_ms).await {
            Ok(out) => {
                let mut content = out.stdout;
                if out.timed_out {
                    // Name the budget that was actually spent and both ways out,
                    // so the model can correct itself instead of re-sending the
                    // same command into the same wall.
                    content.push_str(&format!(
                        "\n[command timed out after {timeout_ms}ms. The output above is partial and \
                         side effects may still be in progress — inspect the remote state before \
                         retrying. Retry with a larger `timeout` (up to {MAX_TIMEOUT_MS}ms), or, if \
                         the work needs longer than that, detach it \
                         (`nohup <command> > /tmp/<name>.log 2>&1 &`) and poll the log file.]"
                    ));
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
        "Read a file from the remote host over SFTP. Reads the whole file, so it is \
         limited to files under 1 MiB — for anything larger (logs, dumps) take a slice \
         with Bash instead, e.g. `sed -n '1,200p' <path>` or `tail -n 200 <path>`."
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
        // Stat first: SFTP `read_file` returns the entire file as one buffer that
        // then gets copied into a String, and both live in the user's desktop
        // process. A directory or an oversized file must be refused before any
        // byte is pulled — with a bounded alternative, so the model can recover.
        match self.backend.stat(path).await {
            Ok(stat) if stat.is_dir => {
                return ToolResult::error(format!(
                    "{path} is a directory, not a file. List it with Bash: `ls -la {path}`."
                ));
            }
            Ok(stat) if stat.size > MAX_REMOTE_READ_BYTES => {
                return ToolResult::error(format!(
                    "{path} is {} bytes, over the {MAX_REMOTE_READ_BYTES}-byte remote Read limit. \
                     Take the part you need with Bash instead: `sed -n '1,200p' {path}` for the head, \
                     `tail -n 200 {path}` for the tail, or Grep to find the relevant lines first.",
                    stat.size
                ));
            }
            Ok(_) => {}
            Err(e) => return ToolResult::error(format!("remote stat failed: {e}")),
        }
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
        "List remote files matching a POSIX shell glob. The remote shell is /bin/sh: only \
         single-level `*`, `?` and `[...]` are supported, and `**` behaves as one `*` — it does \
         NOT recurse. To search a whole tree, use Bash with `find` (e.g. \
         `find <dir> -name '*.rs'`)."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "POSIX glob matched by the remote shell, e.g. `src/*.rs` or `/etc/*.conf`. Not recursive — use Bash `find` for a whole tree."
                }
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
            // An empty string reads as both "no such files" and "the tool broke".
            // Say which one it was.
            Ok(entries) if entries.is_empty() => {
                ToolResult::text(format!("no matches for {pattern}"))
            }
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

    use std::sync::Mutex;

    struct MockBackend {
        last_command: Mutex<Option<String>>,
        last_timeout_ms: Mutex<Option<u64>>,
        timed_out: Mutex<bool>,
        last_stat_path: Mutex<Option<String>>,
        stat: Mutex<crate::ssh_backend::RemoteFileStat>,
        entries: Mutex<Vec<String>>,
        read_calls: Mutex<usize>,
    }

    #[async_trait]
    impl SshBackend for MockBackend {
        async fn run_command(
            &self,
            command: &str,
            timeout_ms: u64,
        ) -> Result<crate::ssh_backend::RemoteCommandOutput, String> {
            *self.last_command.lock().unwrap() = Some(command.to_string());
            *self.last_timeout_ms.lock().unwrap() = Some(timeout_ms);
            Ok(crate::ssh_backend::RemoteCommandOutput {
                stdout: format!("ran: {command}"),
                exit_code: 0,
                timed_out: *self.timed_out.lock().unwrap(),
            })
        }
        async fn read_file(&self, _path: &str) -> Result<Vec<u8>, String> {
            *self.read_calls.lock().unwrap() += 1;
            Ok(b"hello\nworld\n".to_vec())
        }
        async fn write_file(&self, _path: &str, _bytes: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
        async fn grep(&self, _pattern: &str, _path: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn list_files(&self, _glob: &str) -> Result<Vec<String>, String> {
            Ok(self.entries.lock().unwrap().clone())
        }
        async fn stat(
            &self,
            path: &str,
        ) -> Result<crate::ssh_backend::RemoteFileStat, String> {
            *self.last_stat_path.lock().unwrap() = Some(path.to_string());
            Ok(self.stat.lock().unwrap().clone())
        }
    }

    fn mock() -> Arc<MockBackend> {
        Arc::new(MockBackend {
            last_command: Mutex::new(None),
            last_timeout_ms: Mutex::new(None),
            timed_out: Mutex::new(false),
            last_stat_path: Mutex::new(None),
            stat: Mutex::new(crate::ssh_backend::RemoteFileStat { size: 12, is_dir: false }),
            entries: Mutex::new(vec![]),
            read_calls: Mutex::new(0),
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

    // ── Bash timeout (this tool occupies the local `Bash` name, so the model's
    //    trained-in `timeout` argument must mean the same thing here) ──────────

    #[tokio::test]
    async fn bash_forwards_the_model_supplied_timeout_and_clamps_it() {
        let b = mock();
        let tool = SshBashTool::new(b.clone());

        tool.execute(json!({"command": "sleep 1"})).await;
        assert_eq!(
            *b.last_timeout_ms.lock().unwrap(),
            Some(DEFAULT_TIMEOUT_MS),
            "no timeout given must use the same default the local Bash advertises"
        );

        tool.execute(json!({"command": "cargo build", "timeout": 300_000})).await;
        assert_eq!(
            *b.last_timeout_ms.lock().unwrap(),
            Some(300_000),
            "an explicit timeout must reach the backend instead of being dropped"
        );

        tool.execute(json!({"command": "apt-get install -y x", "timeout": 99_999_999})).await;
        assert_eq!(
            *b.last_timeout_ms.lock().unwrap(),
            Some(MAX_TIMEOUT_MS),
            "an over-budget timeout must clamp to the max, not be ignored"
        );
    }

    #[test]
    fn bash_advertises_the_timeout_parameter_with_the_local_budget() {
        let tool = SshBashTool::new(mock());
        let schema = tool.input_schema();
        assert!(
            schema["properties"]["timeout"].is_object(),
            "schema must expose `timeout`: {schema}"
        );
        let described = format!("{}\n{schema}", tool.description());
        assert!(described.contains("120000"), "default budget must be stated: {described}");
        assert!(described.contains("600000"), "max budget must be stated: {described}");
    }

    #[tokio::test]
    async fn bash_timeout_text_states_the_budget_and_a_way_forward() {
        let b = mock();
        *b.timed_out.lock().unwrap() = true;
        let res = SshBashTool::new(b)
            .execute(json!({"command": "cargo build", "timeout": 5_000}))
            .await;
        assert!(
            res.content.contains("5000"),
            "the actual budget must be named so the model can raise it: {}",
            res.content
        );
        assert!(
            res.content.contains("timeout"),
            "the model must be told which parameter to raise: {}",
            res.content
        );
        assert!(
            res.content.contains("nohup"),
            "a command that outlasts the max needs the background escape hatch: {}",
            res.content
        );
    }

    #[tokio::test]
    async fn read_is_info_and_returns_content() {
        let tool = SshReadTool::new(mock());
        assert!(matches!(tool.category(), ToolCategory::Info));
        let res = tool.execute(json!({"file_path": "/etc/hostname"})).await;
        assert!(res.content.contains("hello"));
    }

    // ── Read size gate (an unbounded SFTP read lands in the desktop process) ──

    #[tokio::test]
    async fn read_checks_the_remote_size_before_pulling_bytes() {
        let b = mock();
        SshReadTool::new(b.clone())
            .execute(json!({"file_path": "/etc/hostname"}))
            .await;
        assert_eq!(
            b.last_stat_path.lock().unwrap().as_deref(),
            Some("/etc/hostname"),
            "Read must stat the path before streaming it into memory"
        );
    }

    #[tokio::test]
    async fn read_refuses_a_directory_and_says_how_to_list_it() {
        let b = mock();
        b.stat.lock().unwrap().is_dir = true;
        let res = SshReadTool::new(b.clone())
            .execute(json!({"file_path": "/var/log"}))
            .await;
        assert!(res.is_error, "a directory read must not look like content");
        assert!(res.content.contains("directory"), "got: {}", res.content);
        assert!(res.content.contains("ls"), "must point at a listing: {}", res.content);
        assert_eq!(*b.read_calls.lock().unwrap(), 0, "no bytes may be pulled");
    }

    #[tokio::test]
    async fn read_refuses_an_oversized_file_and_suggests_a_slice() {
        let b = mock();
        b.stat.lock().unwrap().size = MAX_REMOTE_READ_BYTES + 1;
        let res = SshReadTool::new(b.clone())
            .execute(json!({"file_path": "/var/log/syslog"}))
            .await;
        assert!(res.is_error, "an oversized read must be refused: {}", res.content);
        assert!(
            res.content.contains("sed -n"),
            "the model needs a bounded way to get the part it wants: {}",
            res.content
        );
        assert_eq!(
            *b.read_calls.lock().unwrap(),
            0,
            "the whole point is that the bytes never reach this process"
        );
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

    // ── Glob (the remote shell is POSIX sh: `**` is just `*`) ────────────────

    #[test]
    fn glob_never_advertises_a_recursive_pattern_it_cannot_expand() {
        let tool = SshGlobTool::new(mock());
        let described = format!("{}\n{}", tool.description(), tool.input_schema());
        assert!(
            !described.contains("**/"),
            "a copyable `**/…` example is expanded as a single `*` by the remote sh, so deep \
             files silently vanish and look like absence: {described}"
        );
        assert!(
            described.contains("NOT recurse") || described.contains("Not recursive"),
            "the missing globstar must be stated, not left to be discovered: {described}"
        );
        assert!(
            described.contains("find"),
            "the model must be told where recursion actually lives: {described}"
        );
    }

    #[tokio::test]
    async fn glob_says_no_matches_instead_of_returning_an_empty_string() {
        // An empty string reads as both "no such files" and "the tool broke".
        let res = SshGlobTool::new(mock())
            .execute(json!({"pattern": "src/*.rs"}))
            .await;
        assert!(!res.is_error, "no matches is an answer, not a failure");
        assert!(
            res.content.contains("src/*.rs"),
            "the answer must name the pattern it answered: {:?}",
            res.content
        );
        assert!(
            res.content.to_lowercase().contains("no match"),
            "got: {:?}",
            res.content
        );
    }
}
