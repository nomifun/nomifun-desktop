# Windows Shell ConPTY Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent Windows shell commands from creating stray visible `cmd.exe` consoles by running them in ConPTY and rejecting explicit window-launch requests.

**Architecture:** A focused `windows_shell` policy module owns Windows transport selection and command validation. `BashTool` and `ExecCommandTool` consult it before creating an execution request. `nomi-execution` retains ownership of ConPTY, Job Object membership, and lifecycle cleanup.

**Tech Stack:** Rust, Tokio, `nomi-execution`, Windows ConPTY, Cargo tests.

## Global Constraints

- On Windows, every shell request uses `Transport::Pty { cols: 120, rows: 30 }`.
- On macOS/Linux, retain the current `tty`-controlled Pipe/PTY selection.
- Never retry a failed Windows ConPTY startup through Pipe.
- Permit `cmd /c`; reject explicit separate-window or detached launch forms before spawning.
- Do not modify Job Object, Unix watchdog, or process-group lifecycle without a failing lifecycle regression.

---

### Task 1: Isolate Windows shell transport and launch policy

**Files:**

- Create: `crates/agent/nomi-tools/src/windows_shell.rs`
- Modify: `crates/agent/nomi-tools/src/lib.rs`
- Test: inline module tests in `crates/agent/nomi-tools/src/windows_shell.rs`

**Interfaces:**

- Produces `pub(crate) fn shell_transport(requested_tty: bool) -> Transport`.
- Produces `pub(crate) fn validate_shell_script(script: &str) -> Result<(), String>`.
- Consumed by `BashTool::run_supervised` and `requested_invocation`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn windows_shell_uses_pty_even_when_tty_is_not_requested() {
    let transport = shell_transport(false);
    #[cfg(windows)] assert_eq!(transport, Transport::Pty { cols: 120, rows: 30 });
    #[cfg(not(windows))] assert_eq!(transport, Transport::Pipe);
}

#[test]
fn rejects_explicit_windows_launch_forms_but_allows_cmd_c() {
    #[cfg(windows)] {
        assert!(validate_shell_script("start cmd").is_err());
        assert!(validate_shell_script("Start-Process notepad").is_err());
        assert!(validate_shell_script("cmd /k echo hi").is_err());
    }
    assert!(validate_shell_script("cmd /c echo hi").is_ok());
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p nomi-tools windows_shell --lib`. Expected: compile failure because the module does not exist.

- [ ] **Step 3: Implement the smallest module**

```rust
pub(crate) const SHELL_PTY_COLS: u16 = 120;
pub(crate) const SHELL_PTY_ROWS: u16 = 30;

pub(crate) fn shell_transport(requested_tty: bool) -> Transport {
    if cfg!(windows) || requested_tty { Transport::Pty { cols: SHELL_PTY_COLS, rows: SHELL_PTY_ROWS } }
    else { Transport::Pipe }
}
```

Implement a Windows-only, case-insensitive, quote-aware command-token scanner. Reject command tokens `start`, `start-process`, and `cmd` followed by `/k`; do not reject quoted data or `cmd /c`. Return: `Windows shell commands cannot open a separate console or application window; use the dedicated launch tool`.

- [ ] **Step 4: Verify GREEN**

Run `cargo test -p nomi-tools windows_shell --lib`. Expected: PASS.

- [ ] **Step 5: Commit**

Run `git add crates/agent/nomi-tools/src/windows_shell.rs crates/agent/nomi-tools/src/lib.rs && git commit -m "feat(tools): add Windows shell console policy"`.

### Task 2: Route Bash through the policy

**Files:**

- Modify: `crates/agent/nomi-tools/src/bash.rs:50-82, 242-272`
- Test: `crates/agent/nomi-tools/src/bash.rs:561-1000`

**Interfaces:**

- Consumes `shell_transport(false)` and `validate_shell_script(&command)`.
- Returns a `ToolResult::error` before `ProcessSupervisor::start` for policy failures.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn windows_bash_rejects_explicit_window_launch_before_execution() {
    let result = tool(std::env::temp_dir()).execute(json!({"command": "Start-Process notepad"})).await;
    #[cfg(windows)] assert!(result.is_error && result.content.contains("cannot open a separate console"));
}

#[tokio::test]
async fn windows_bash_runs_normal_cmd_c_without_a_pipe_transport() {
    let result = tool(std::env::temp_dir()).execute(json!({"command": "cmd /c echo conpty_marker"})).await;
    #[cfg(windows)] assert!(result.content.contains("conpty_marker"), "{}", result.content);
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p nomi-tools windows_bash --lib`. Expected: launch rejection fails on Windows because Bash accepts the command.

- [ ] **Step 3: Implement**

After extracting `command` in `BashTool::execute`, return `ToolResult::error(error)` if validation fails. Replace the request's `Transport::Pipe` with `shell_transport(false)`. Do not add a Pipe fallback.

- [ ] **Step 4: Verify GREEN**

Run `cargo test -p nomi-tools bash::tests --lib`. Expected: PASS on the host; Windows assertions run on Windows.

- [ ] **Step 5: Commit**

Run `git add crates/agent/nomi-tools/src/bash.rs && git commit -m "fix(tools): isolate Windows Bash consoles in ConPTY"`.

### Task 3: Route exec_command shell modes through the policy

**Files:**

- Modify: `crates/agent/nomi-tools/src/exec_command.rs:741-853`
- Test: `crates/agent/nomi-tools/src/exec_command.rs:1201-1759`

**Interfaces:**

- Legacy mode consumes `shell_transport(tty)`.
- Shell-script mode consumes `shell_transport(false)`.
- Python script mode retains `Transport::Pipe`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn windows_legacy_noninteractive_shell_uses_pty() {
    let prepared = requested_invocation(&json!({"cmd": "Write-Output ok", "tty": false})).unwrap();
    #[cfg(windows)] assert_eq!(prepared.transport, Transport::Pty { cols: 120, rows: 30 });
    #[cfg(not(windows))] assert_eq!(prepared.transport, Transport::Pipe);
}

#[test]
fn windows_shell_script_uses_pty_and_rejects_cmd_k() {
    let prepared = requested_invocation(&json!({"script": "Write-Output ok", "language": "shell", "timeout": 1000})).unwrap();
    #[cfg(windows)] assert_eq!(prepared.transport, Transport::Pty { cols: 120, rows: 30 });
    #[cfg(windows)] assert!(requested_invocation(&json!({"cmd": "cmd /k echo hi"})).is_err());
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p nomi-tools windows_legacy_noninteractive_shell_uses_pty --lib`. Expected: failure on Windows because `tty: false` chooses Pipe.

- [ ] **Step 3: Implement**

Validate legacy `cmd` after parsing it and validate `language: "shell"` source after parsing it. Replace legacy Pipe/TTY selection with `shell_transport(tty)` and shell-script Pipe with `shell_transport(false)`. Keep Python script transport unchanged. Mark any separate stdout/stderr attribution assertion as non-Windows, because PTY output is a single stream.

- [ ] **Step 4: Verify GREEN**

Run `cargo test -p nomi-tools exec_command::tests --lib`. Expected: PASS.

- [ ] **Step 5: Commit**

Run `git add crates/agent/nomi-tools/src/exec_command.rs && git commit -m "fix(tools): use ConPTY for Windows shell sessions"`.

### Task 4: Prove cleanup and verify scope

**Files:**

- Modify: `crates/agent/nomi-tools/src/bash.rs:561-1000`
- Modify: `crates/agent/nomi-tools/src/exec_command.rs:1201-1759`
- Test: existing Windows tests in `crates/shared/nomi-execution/src/platform/windows.rs` only if a kernel defect is demonstrated

**Interfaces:**

- Consumes existing `ProcessSupervisor`, Job Object cleanup, and `pty_test_helper` process fixture.
- Produces a Windows-gated test that starts a `cmd /c` descendant and proves timeout/cancellation reaps it.

- [ ] **Step 1: Write the failing regression**

```rust
#[cfg(windows)]
#[tokio::test]
async fn windows_timeout_reaps_cmd_descendant_started_in_conpty() {
    let result = tool(std::env::temp_dir())
        .execute(json!({"command": "cmd /c ping -n 60 127.0.0.1 > nul", "timeout": 250}))
        .await;
    assert!(result.is_error, "{}", result.content);
    assert!(result.content.to_ascii_lowercase().contains("timed out"));
}
```

Prefer the existing helper PID/marker fixture if descendant identity must be observed. Desktop-window visibility remains an interactive Windows smoke test; CI proves transport and cleanup, not pixels.

- [ ] **Step 2: Verify the regression**

Run `cargo test -p nomi-tools windows_timeout_reaps_cmd_descendant_started_in_conpty --lib` on Windows. Expected after Tasks 2–3: PASS.

- [ ] **Step 3: Full verification**

Run `cargo test -p nomi-tools --lib` and `cargo test -p nomi-execution --lib`. Expected: PASS. On an interactive Windows host, run one `cmd /c` task and verify no console window appears; cancel a long-running child and verify it exits.

- [ ] **Step 4: Commit**

Run `git add crates/agent/nomi-tools/src/bash.rs crates/agent/nomi-tools/src/exec_command.rs crates/shared/nomi-execution/src/platform/windows.rs && git commit -m "test(tools): cover Windows shell console cleanup"`.
