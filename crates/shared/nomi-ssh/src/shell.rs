//! `RemoteShell`: one long-lived remote shell over a PTY channel whose cwd and
//! environment persist across sequential commands — the remote analogue of a
//! real interactive session, not a fresh `ssh host cmd` per call.
//!
//! # Completion protocol (controlled sentinel)
//!
//! After each submitted command the shell prints a unique sentinel line
//! carrying the command's exit status **and its cwd**:
//!
//! ```text
//! <script>
//! printf '__NOMI_END_<nonce>__%d__%s\n' "$?" "$PWD"
//! ```
//!
//! We read PTY output until `__NOMI_END_<nonce>__<rc>__<pwd>\n` appears;
//! everything before it is the command's output, `<rc>` is the exit code, and
//! `<pwd>` is the shell's cwd after the command (used to restore cwd on
//! reconnect). Input echo is disabled (`stty -echo`) and prompts are blanked at
//! init so captured output is only the command's own stdout/stderr. This is the
//! standard technique used by persistent-shell coding tools; detection is exact,
//! not heuristic. `find_sentinel` skips the first occurrence so an echoed
//! command line can never be mistaken for the real marker.
//!
//! The shell is line-oriented and we control its command line fully. To avoid
//! quoting/injection and multi-line-prompt bugs, callers should upload a script
//! and run one line (`bash <path>`); this module accepts any single submission
//! string and appends the sentinel.
use std::sync::Arc;
use std::time::Duration;

use russh::ChannelMsg;
use russh::client::Msg;
use tokio::sync::Mutex;

use crate::connection::{SshConnection, SshError};
use crate::responder::AnswerRule;

/// Ctrl-C (ETX): interrupts the foreground command on a PTY.
const CTRL_C: u8 = 0x03;
/// How long to wait for the shell to reach its first ready sentinel at spawn.
const INIT_READY_TIMEOUT: Duration = Duration::from_millis(5_000);
/// After an interrupt on timeout, how long to wait for the fresh drain sentinel
/// to flush before declaring the shell unrecoverable. Generous because the
/// interrupted command's own late sentinel is drained first.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(3_000);

/// Outcome of running one command in the persistent remote shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutcome {
    /// Combined stdout/stderr (PTY-interleaved), carriage returns stripped.
    pub output: String,
    /// The command's exit code.
    pub exit_code: i32,
    /// The shell's cwd after the command (from the sentinel), for reconnect.
    pub cwd: String,
    /// True when the command did not finish within the timeout.
    pub timed_out: bool,
}

/// A long-lived remote shell whose cwd/env persist across `run` calls. Commands
/// are serialized through an internal lock (a single shell process cannot
/// interleave commands), so this is cheap to share via `Arc`.
pub struct RemoteShell {
    seq: std::sync::atomic::AtomicU64,
    channel: Mutex<russh::Channel<Msg>>,
    /// Prompt-driven auto-answers (sudo password, apt y/n, ...). Injected during
    /// `run`; answers are written to input only, never captured.
    answer_rules: Vec<AnswerRule>,
}

impl SshConnection {
    /// Open a persistent shell rooted at `cwd`. Requests a PTY + shell, blanks
    /// the prompt, disables echo, cds into `cwd`, and drains init noise via a
    /// priming sentinel.
    pub async fn open_shell(&self, cwd: &str) -> Result<Arc<RemoteShell>, SshError> {
        self.open_shell_with_rules(cwd, Vec::new()).await
    }

    /// Like [`open_shell`](Self::open_shell) but installs prompt-driven
    /// auto-answer rules (e.g. a sudo password) that inject into the shell's
    /// input during `run`. Answers are written to input only and never captured.
    pub async fn open_shell_with_rules(
        &self,
        cwd: &str,
        answer_rules: Vec<AnswerRule>,
    ) -> Result<Arc<RemoteShell>, SshError> {
        let channel = self.handle().channel_open_session().await?;
        channel
            .request_pty(true, "xterm-256color", 200, 50, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;

        let shell = RemoteShell {
            seq: std::sync::atomic::AtomicU64::new(1),
            channel: Mutex::new(channel),
            answer_rules,
        };

        // Init. `request_shell` starts the operator's *interactive login shell*,
        // which sources rc files that enable bracketed-paste mode, OSC shell-
        // integration markers, and PROMPT_COMMAND — all of which corrupt captured
        // output and, after a Ctrl-C, the next command's input. So first `exec
        // /bin/sh` to drop into a clean POSIX shell (no readline, no brackets, no
        // OSC), then blank prompts, disable echo, cd, and prime sentinel 0. The
        // PTY byte stream is FIFO: bash reads the `exec` line and replaces itself;
        // the new sh reads the remaining buffered init bytes.
        let init = format!(
            "exec /bin/sh\nstty -echo 2>/dev/null; PS1=''; PS2=''; unset PROMPT_COMMAND 2>/dev/null; cd {} 2>/dev/null\n",
            shell_quote(cwd)
        );
        {
            let mut ch = shell.channel.lock().await;
            ch.data_bytes(init.into_bytes()).await?;
            ch.data_bytes(sentinel_command(0).into_bytes()).await?;
            let mut sink = String::new();
            if collect_until_sentinel(&mut ch, &sentinel_prefix(0), INIT_READY_TIMEOUT, &mut sink, &[])
                .await
                .is_none()
            {
                return Err(SshError::Protocol(
                    "remote shell did not become ready".into(),
                ));
            }
        }
        Ok(Arc::new(shell))
    }
}

impl RemoteShell {
    /// Run `submission` (typically `bash <uploaded-script>`), returning output,
    /// exit code, and post-command cwd. cwd/env mutations persist. On timeout the
    /// foreground command is interrupted (Ctrl-C → SIGINT → SIGTERM); if the
    /// aborted command's sentinel can be resynced, `timed_out` is set with the
    /// real code, else `exit_code = 124`.
    pub async fn run(&self, submission: &str, timeout: Duration) -> Result<ShellOutcome, SshError> {
        let nonce = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let prefix = sentinel_prefix(nonce);
        let mut ch = self.channel.lock().await;

        // Join the command and its sentinel with `;` on ONE input line (not two
        // lines). A command that reads stdin/tty (sudo, `read`) would otherwise
        // consume the sentinel line buffered behind it. On one line the shell
        // parses the whole list first, leaving the input buffer empty for the
        // interactive command to receive the responder's injected answer.
        let payload = format!("{submission}; {}", sentinel_command(nonce));
        ch.data_bytes(payload.into_bytes()).await?;

        let mut buf = String::new();
        match collect_until_sentinel(&mut ch, &prefix, timeout, &mut buf, &self.answer_rules).await {
            Some((rc, cwd)) => {
                let start = find_sentinel(&buf, &prefix).map(|(s, _, _)| s).unwrap_or(buf.len());
                Ok(ShellOutcome {
                    output: clean(&buf[..start]),
                    exit_code: rc,
                    cwd,
                    timed_out: false,
                })
            }
            None => {
                // Timed out. Interrupt the foreground command, then actively
                // *drain* the channel by emitting a fresh sentinel probe and
                // reading until it appears. This consumes the aborted command's
                // own late sentinel and any trailing bytes, so the channel is
                // clean for the next command — a plain resync-to-old-nonce is
                // unreliable under load and leaves stale bytes that corrupt the
                // next submission. Matches how mature persistent-shell tools
                // recover from an interrupt.
                let partial = extract_output(&buf, &prefix);
                ch.data_bytes(vec![CTRL_C]).await.ok();

                let drain_nonce = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let drain_prefix = sentinel_prefix(drain_nonce);
                ch.data_bytes(sentinel_command(drain_nonce).into_bytes())
                    .await
                    .ok();

                let mut drain = String::new();
                if let Some((_, cwd)) =
                    collect_until_sentinel(&mut ch, &drain_prefix, DRAIN_TIMEOUT, &mut drain, &[]).await
                {
                    // Interrupt succeeded and the shell is resynchronized.
                    return Ok(ShellOutcome {
                        output: partial,
                        exit_code: 124, // conventional timeout code
                        cwd,
                        timed_out: true,
                    });
                }

                // Drain failed: the shell is unrecoverable. Escalate signals so
                // the remote process is asked to die, and report an honest,
                // cwd-less timeout — the caller/pool should recycle the shell.
                ch.signal(russh::Sig::INT).await.ok();
                ch.signal(russh::Sig::TERM).await.ok();
                Ok(ShellOutcome {
                    output: partial,
                    exit_code: 124,
                    cwd: String::new(),
                    timed_out: true,
                })
            }
        }
    }
}

/// The `printf` that emits sentinel `nonce` carrying the prior command's `$?`
/// and cwd. cwd is last and terminated by the line's newline (paths contain no
/// newline), so parsing is unambiguous.
fn sentinel_command(nonce: u64) -> String {
    format!("printf '__NOMI_END_{nonce}__%d__%s\\n' \"$?\" \"$PWD\"\n")
}

fn sentinel_prefix(nonce: u64) -> String {
    format!("__NOMI_END_{nonce}__")
}

/// Read from the channel into `sink` until a parseable sentinel for `prefix`
/// appears or `timeout` elapses / the channel closes. Returns `(exit_code, cwd)`.
///
/// While reading, `answer_rules` are matched against freshly-arrived output and
/// their answers injected into the shell's input (once each, if `once`). The
/// answer bytes are written to the channel only — never pushed into `sink` — so
/// a sudo password cannot appear in captured output.
async fn collect_until_sentinel(
    ch: &mut russh::Channel<Msg>,
    prefix: &str,
    timeout: Duration,
    sink: &mut String,
    answer_rules: &[AnswerRule],
) -> Option<(i32, String)> {
    if let Some((_, rc, cwd)) = find_sentinel(sink, prefix) {
        return Some((rc, cwd));
    }
    let mut fired = vec![false; answer_rules.len()];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, ch.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                sink.push_str(&String::from_utf8_lossy(&data));
                if let Some((_, rc, cwd)) = find_sentinel(sink, prefix) {
                    return Some((rc, cwd));
                }
                maybe_inject_answers(ch, sink, answer_rules, &mut fired).await;
            }
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                // A PTY normally folds stderr into Data, but be tolerant.
                sink.push_str(&String::from_utf8_lossy(&data));
                if let Some((_, rc, cwd)) = find_sentinel(sink, prefix) {
                    return Some((rc, cwd));
                }
                maybe_inject_answers(ch, sink, answer_rules, &mut fired).await;
            }
            // Non-output messages (WindowAdjusted, Success, ...) — keep waiting.
            Ok(Some(_)) => continue,
            // Channel closed (shell died) or timeout — give up.
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Check each not-yet-fired rule against the accumulated output; on a match,
/// write `answer\n` to the shell input and mark the rule fired (if `once`).
async fn maybe_inject_answers(
    ch: &russh::Channel<Msg>,
    sink: &str,
    answer_rules: &[AnswerRule],
    fired: &mut [bool],
) {
    for (i, rule) in answer_rules.iter().enumerate() {
        if fired[i] {
            continue;
        }
        if rule.prompt.is_match(sink) {
            let mut bytes = rule.answer.as_bytes().to_vec();
            bytes.push(b'\n');
            let _ = ch.data_bytes(bytes).await;
            if rule.once {
                fired[i] = true;
            }
        }
    }
}

/// Scan all occurrences of `prefix`; return the byte offset of the first one
/// followed by `<digits>__<pwd>\n` (the real sentinel), plus the code and cwd.
/// Earlier occurrences with a non-numeric tail (an echoed command line) are
/// skipped, so detection is robust even if `stty -echo` didn't take.
fn find_sentinel(buf: &str, prefix: &str) -> Option<(usize, i32, String)> {
    let mut search_from = 0;
    while let Some(rel) = buf[search_from..].find(prefix) {
        let start = search_from + rel;
        let after = &buf[start + prefix.len()..];
        if let Some(sep) = after.find("__")
            && let Ok(rc) = after[..sep].parse::<i32>()
        {
            let rest = &after[sep + 2..];
            if let Some(nl) = rest.find('\n') {
                let cwd = rest[..nl].trim_end_matches('\r').to_owned();
                return Some((start, rc, cwd));
            }
        }
        search_from = start + prefix.len();
    }
    None
}

/// Output before the sentinel, used on the timeout path where no code parsed.
fn extract_output(buf: &str, prefix: &str) -> String {
    match find_sentinel(buf, prefix)
        .map(|(s, _, _)| s)
        .or_else(|| buf.find(prefix))
    {
        Some(start) => clean(&buf[..start]),
        None => clean(buf),
    }
}

/// Strip carriage returns and a single trailing newline from captured output.
fn clean(s: &str) -> String {
    let s = s.replace('\r', "");
    s.strip_suffix('\n').unwrap_or(&s).to_owned()
}

/// Minimal single-quote shell quoting for the init `cd` path.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
