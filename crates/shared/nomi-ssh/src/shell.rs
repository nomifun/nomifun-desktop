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
//! not heuristic. `find_sentinel` skips non-numeric format strings in echoed
//! commands; only a marker with a parseable status and cwd terminates a read.
//!
//! The shell is line-oriented and we control its command line fully. To avoid
//! quoting/injection and multi-line-prompt bugs, callers should upload a script
//! and run one line (`bash <path>`); this module accepts any single submission
//! string and appends the sentinel.
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use russh::ChannelMsg;
use russh::client::Msg;
use tokio::sync::Mutex;
use tokio::time::{Instant, timeout_at};

use crate::connection::{SshConnection, SshError};
use crate::limits::{MAX_SSH_OUTPUT_BYTES, validate_command};
use crate::responder::AnswerRule;

#[path = "shell_output.rs"]
mod output;
use output::ShellOutput;

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

/// Why [`collect_until_sentinel`] stopped reading. `Closed` and `TimedOut` used
/// to be indistinguishable (both `None`), which made a dead link look like a
/// slow command and left `exit_code: 124` as the only hint.
enum SentinelEnd {
    Found { exit_code: i32, cwd: String },
    TimedOut,
    Closed,
    OutputLimitExceeded { actual: usize },
}

/// Evidence gathered while closing the shell channel. `exit_status` /
/// `exit_signal` are the only proof that the remote shell was *reaped* rather
/// than merely abandoned, so teardown reporting reads them instead of assuming.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellCloseProof {
    /// We sent EOF on the channel (a request, not evidence by itself).
    pub eof_sent: bool,
    /// The server closed the channel, or its message stream ended.
    pub channel_closed: bool,
    /// `exit-status` reported by the server for the shell.
    pub exit_status: Option<u32>,
    /// `exit-signal` reported by the server, when the shell died on a signal.
    pub exit_signal: Option<String>,
    /// Everything that went wrong while trying to obtain proof. Non-empty here
    /// is how an unproven close explains itself instead of going quiet.
    pub errors: Vec<String>,
}

impl ShellCloseProof {
    /// True only when the channel closed AND the remote reported how the shell
    /// ended. Anything else is unproven and must be reported as lost — never
    /// upgraded to reaped because cleanup "probably" worked.
    pub fn is_reaped(&self) -> bool {
        self.channel_closed && (self.exit_status.is_some() || self.exit_signal.is_some())
    }
}

/// A long-lived remote shell whose cwd/env persist across `run` calls. Commands
/// are serialized through an internal lock (a single shell process cannot
/// interleave commands), so this is cheap to share via `Arc`.
pub struct RemoteShell {
    seq: std::sync::atomic::AtomicU64,
    /// Taken for the duration of an operation. Cancellation retires the channel
    /// from command reuse but preserves its receiver for explicit close proof.
    channel: ChannelSlot,
    operation: Mutex<()>,
    /// Prompt-driven auto-answers (sudo password, apt y/n, ...). Injected during
    /// `run`; answers are written to input only, never captured.
    answer_rules: Vec<AnswerRule>,
}

#[derive(Clone, Copy)]
enum ChannelUnavailable {
    UnknownOutcome,
    Disconnected,
}

impl ChannelUnavailable {
    fn error(self) -> SshError {
        match self {
            Self::UnknownOutcome => SshError::Protocol(
                "remote shell channel is unavailable after cancellation or failed recovery".into(),
            ),
            Self::Disconnected => SshError::Disconnected(
                "remote shell channel has already closed".into(),
            ),
        }
    }
}

/// Ownership and availability are one atomic state update. A retired channel
/// stays here only for close evidence; it can never be leased by run again.
struct ShellChannelState {
    channel: Option<russh::Channel<Msg>>,
    reusable: bool,
    unavailable: ChannelUnavailable,
}

type ChannelSlot = std::sync::Mutex<ShellChannelState>;

struct OperationChannel<'a> {
    channel: Option<russh::Channel<Msg>>,
    retired: Option<&'a ChannelSlot>,
}

impl<'a> OperationChannel<'a> {
    fn new(channel: russh::Channel<Msg>, retired: Option<&'a ChannelSlot>) -> Self {
        Self { channel: Some(channel), retired }
    }

    fn get_mut(&mut self) -> &mut russh::Channel<Msg> {
        self.channel
            .as_mut()
            .expect("operation channel is present until returned")
    }

    fn take_reusable(&mut self) -> russh::Channel<Msg> {
        self.channel
            .take()
            .expect("operation channel can only be returned once")
    }
}

impl Drop for OperationChannel<'_> {
    fn drop(&mut self) {
        let Some(channel) = self.channel.take() else {
            return;
        };
        retire_channel(&channel);
        if let Some(retired) = self.retired {
            let mut slot = retired.lock().unwrap();
            slot.channel = Some(channel);
            slot.reusable = false;
        }
    }
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
        crate::limits::validate_path(cwd).map_err(limit_error)?;
        let deadline = Instant::now() + INIT_READY_TIMEOUT;
        timeout_at(deadline, self.initialize_shell(cwd, answer_rules, deadline))
            .await
            .map_err(|_| SshError::TimedOut("remote shell initialization exceeded its budget".into()))?
    }

    async fn initialize_shell(
        &self,
        cwd: &str,
        answer_rules: Vec<AnswerRule>,
        deadline: Instant,
    ) -> Result<Arc<RemoteShell>, SshError> {
        let mut leased = OperationChannel::new(self.handle().channel_open_session().await?, None);
        let channel = leased.get_mut();
        channel
            .request_pty(true, "xterm-256color", 200, 50, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;

        // Init. `request_shell` starts the operator's *interactive login shell*,
        // which sources rc files that enable bracketed-paste mode, OSC shell-
        // integration markers, and PROMPT_COMMAND — all of which corrupt captured
        // output and, after a Ctrl-C, the next command's input. So first `exec
        // /bin/sh` to drop into a clean POSIX shell (no readline, no brackets, no
        // OSC), then blank prompts, disable echo, cd, and prime sentinel 0. The
        // PTY byte stream is FIFO: bash reads the `exec` line and replaces itself;
        // the new sh reads the remaining buffered init bytes.
        //
        // The pager variables are the other half of "this is a real PTY": sudo
        // needs one, but so every tool that pages when stdout is a tty starts
        // `less` and blocks on a keypress nobody will ever send — `git log`,
        // `git diff`, `systemctl status`, `journalctl`, `man`. The command then
        // burns its whole budget and comes back as a timeout with a screenful of
        // escape codes. `cat` is the value both git and systemd read as "no
        // pager", and `TERM=dumb` stops the rest from emitting cursor control
        // into the capture. sudo is unaffected: it decides whether it has a
        // terminal with `isatty`, not from `TERM`.
        let init = format!(
            "exec /bin/sh\nstty -echo 2>/dev/null; PS1=''; PS2=''; unset PROMPT_COMMAND 2>/dev/null; \
             export PAGER=cat GIT_PAGER=cat SYSTEMD_PAGER=cat TERM=dumb; {} 2>/dev/null\n",
            change_directory_command(cwd)
        );
        channel.data_bytes(init.into_bytes()).await?;
        channel.data_bytes(sentinel_command(0).into_bytes()).await?;
        let mut sink = ShellOutput::default();
        match collect_until_sentinel(
            channel,
            &sentinel_prefix(0),
            deadline,
            &mut sink,
            &[],
        )
        .await
        {
            SentinelEnd::Found { exit_code: 0, .. } => {}
            SentinelEnd::Found { exit_code, .. } => {
                return Err(SshError::InvalidInput(format!(
                    "cannot enter SSH working directory {cwd:?}: cd exited with status {exit_code}"
                )));
            }
            SentinelEnd::TimedOut => {
                return Err(SshError::TimedOut(
                    "remote shell did not become ready".into(),
                ));
            }
            SentinelEnd::Closed => {
                return Err(SshError::Disconnected(
                    "remote shell closed during initialization".into(),
                ));
            }
            SentinelEnd::OutputLimitExceeded { actual } => {
                return Err(SshError::InvalidInput(format!(
                    "SSH output is {actual} bytes; maximum is {MAX_SSH_OUTPUT_BYTES}"
                )));
            }
        }
        let shell = RemoteShell {
            seq: std::sync::atomic::AtomicU64::new(1),
            channel: std::sync::Mutex::new(ShellChannelState {
                channel: Some(leased.take_reusable()),
                reusable: true,
                unavailable: ChannelUnavailable::UnknownOutcome,
            }),
            operation: Mutex::new(()),
            answer_rules,
        };
        Ok(Arc::new(shell))
    }
}

impl RemoteShell {
    /// Run `submission` (typically `bash <uploaded-script>`), returning output,
    /// exit code, and post-command cwd. cwd/env mutations persist. The budget
    /// covers admission, writing and output/answers. A read timeout adds at most
    /// DRAIN_TIMEOUT for interruption/resync and returns exit_code 124. Partial
    /// submission timeout is an error with unknown outcome, never retried.
    pub async fn run(&self, submission: &str, timeout: Duration) -> Result<ShellOutcome, SshError> {
        validate_command(submission).map_err(limit_error)?;
        let deadline = Instant::now() + timeout;
        let _operation = timeout_at(deadline, self.operation.lock()).await
            .map_err(|_| SshError::TimedOut("waiting for the shell command slot".into()))?;
        let channel = {
            let mut slot = self.channel.lock().unwrap();
            if !slot.reusable {
                return Err(slot.unavailable.error());
            }
            slot.reusable = false;
            slot.unavailable = ChannelUnavailable::UnknownOutcome;
            slot.channel.take().expect("reusable shell owns a channel")
        };
        let mut leased = OperationChannel::new(channel, Some(&self.channel));
        let nonce = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let prefix = sentinel_prefix(nonce);

        // Join the command and its sentinel with `;` on ONE input line (not two
        // lines). A command that reads stdin/tty (sudo, `read`) would otherwise
        // consume the sentinel line buffered behind it. On one line the shell
        // parses the whole list first, leaving the input buffer empty for the
        // interactive command to receive the responder's injected answer.
        let payload = format!("{submission}; {}", sentinel_command(nonce));
        // russh only fails this send once its session task is gone, i.e. the
        // link is dead — that is a disconnect, not a protocol violation.
        let sent = timeout_at(deadline, leased.get_mut().data_bytes(payload.into_bytes()))
            .await
            .map_err(|_| SshError::TimedOut("shell submission was not fully written; outcome unknown".into()))?;
        if let Err(error) = sent {
            self.channel.lock().unwrap().unavailable = ChannelUnavailable::Disconnected;
            return Err(SshError::Disconnected(format!(
                "shell channel write failed: {error}"
            )));
        }

        let mut buf = ShellOutput::default();
        let result = match collect_until_sentinel(
            leased.get_mut(),
            &prefix,
            deadline,
            &mut buf,
            &self.answer_rules,
        )
        .await
        {
            SentinelEnd::Found { exit_code, cwd } => {
                let start = find_sentinel(&buf, &prefix).map(|(s, _, _)| s).unwrap_or(buf.len());
                Ok(ShellOutcome {
                    output: clean(&buf[..start]),
                    exit_code,
                    cwd,
                    timed_out: false,
                })
            }
            // The shell is gone. There is no outcome to report and no point
            // interrupting anything: say so, and let the pool redial.
            SentinelEnd::Closed => Err(SshError::Disconnected(
                "remote shell channel closed while awaiting the command sentinel".into(),
            )),
            SentinelEnd::OutputLimitExceeded { actual } => Err(SshError::InvalidInput(format!(
                "SSH output is {actual} bytes; maximum is {MAX_SSH_OUTPUT_BYTES}"
            ))),
            SentinelEnd::TimedOut => {
                // Timed out. Interrupt the foreground command, then actively
                // *drain* the channel by emitting a fresh sentinel probe and
                // reading until it appears. This consumes the aborted command's
                // own late sentinel and any trailing bytes, so the channel is
                // clean for the next command — a plain resync-to-old-nonce is
                // unreliable under load and leaves stale bytes that corrupt the
                // next submission. Matches how mature persistent-shell tools
                // recover from an interrupt.
                let partial = extract_output(&buf, &prefix);
                let drain_nonce = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let drain_prefix = sentinel_prefix(drain_nonce);
                let drain_deadline = Instant::now() + DRAIN_TIMEOUT;
                let mut drain = ShellOutput::default();
                // The recovery allowance includes every write, not only reads.
                let recovered = timeout_at(drain_deadline, async {
                    leased.get_mut().data_bytes(vec![CTRL_C]).await.ok();
                    // Satisfy a pending tty read if Ctrl-C did not interrupt it.
                    leased.get_mut().data_bytes(vec![b'\n']).await.ok();
                    leased.get_mut().data_bytes(sentinel_command(drain_nonce).into_bytes()).await.ok();
                    collect_until_sentinel(leased.get_mut(), &drain_prefix, drain_deadline, &mut drain, &[]).await
                }).await.unwrap_or(SentinelEnd::TimedOut);
                match recovered {
                    SentinelEnd::Found { cwd, .. } => Ok(ShellOutcome {
                        output: partial,
                        exit_code: 124,
                        cwd,
                        timed_out: true,
                    }),
                    SentinelEnd::Closed => Err(SshError::Disconnected(
                        "remote shell channel closed while recovering from a timeout".into(),
                    )),
                    // The lease retires an unrecoverable channel; do not add
                    // unbounded signal writes after the recovery deadline.
                    SentinelEnd::TimedOut => Ok(ShellOutcome {
                        output: partial,
                        exit_code: 124,
                        cwd: String::new(),
                        timed_out: true,
                    }),
                    SentinelEnd::OutputLimitExceeded { actual } => Err(SshError::InvalidInput(
                        format!("SSH output is {actual} bytes; maximum is {MAX_SSH_OUTPUT_BYTES}"),
                    )),
                }
            }
        };

        let reusable = result
            .as_ref()
            .is_ok_and(|outcome| !outcome.timed_out || !outcome.cwd.is_empty());
        if reusable {
            let mut slot = self.channel.lock().unwrap();
            slot.channel = Some(leased.take_reusable());
            slot.reusable = true;
        } else if matches!(&result, Err(SshError::Disconnected(_))) {
            self.channel.lock().unwrap().unavailable = ChannelUnavailable::Disconnected;
        }
        result
    }

    /// Whether this shell has a synchronized channel ready for another explicit
    /// command. False after cancellation or failed recovery until it is replaced.
    pub async fn is_reusable(&self) -> bool {
        self.channel.lock().unwrap().reusable
    }

    /// Close the shell and collect evidence of what happened to it. Never fails:
    /// the returned proof either shows the channel closed with an exit status /
    /// signal (reaped) or records why no proof could be obtained (lost).
    ///
    /// One `budget` covers lock admission, exit/EOF writes, closing messages and
    /// the final close request; no phase restarts the deadline.
    pub async fn close(&self, budget: Duration) -> ShellCloseProof {
        let mut proof = ShellCloseProof::default();
        let deadline = Instant::now() + budget;
        let closing = async {
            let _operation = self.operation.lock().await;
            let taken = {
                let mut slot = self.channel.lock().unwrap();
                let synchronized = slot.reusable;
                slot.reusable = false;
                slot.channel.take().map(|channel| (channel, synchronized))
            };
            let Some((channel, synchronized)) = taken else {
                proof.errors.push("shell channel already unavailable".into());
                return;
            };
            let mut leased = OperationChannel::new(channel, Some(&self.channel));
            let ch = leased.get_mut();
            if synchronized {
                if let Err(e) = ch.data_bytes(b"exit\n".to_vec()).await {
                    proof.errors.push(format!("exit write failed: {e}"));
                }
            } else if let Err(e) = ch.signal(russh::Sig::TERM).await {
                // Never append shell text to a partially submitted command.
                proof.errors.push(format!("termination signal failed: {e}"));
            }
            match ch.eof().await {
                Ok(()) => proof.eof_sent = true,
                Err(e) => proof.errors.push(format!("eof failed: {e}")),
            }
            loop {
                match ch.wait().await {
                    Some(ChannelMsg::ExitStatus { exit_status }) => proof.exit_status = Some(exit_status),
                    Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                        proof.exit_signal = Some(format!("{signal_name:?}"));
                    }
                    Some(ChannelMsg::Close) => {
                        proof.channel_closed = true;
                        break;
                    }
                    Some(_) => continue,
                    None => {
                        proof.channel_closed = true;
                        proof.errors.push("channel stream ended without a close message".into());
                        break;
                    }
                }
            }
            if let Err(e) = ch.close().await {
                proof.errors.push(format!("channel close failed: {e}"));
            }
            // Explicit close finished; do not repeat the retirement requests.
            drop(leased.take_reusable());
        };
        if timeout_at(deadline, closing).await.is_err() {
            proof.errors.push("shell close exceeded its total budget".into());
        }
        proof
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
/// appears, `timeout` elapses, or the channel closes — the three cases the
/// caller must tell apart (see [`SentinelEnd`]).
///
/// While reading, `answer_rules` are matched against freshly-arrived output and
/// their answers injected into the shell's input (once each, if `once`). The
/// answer bytes are written to the channel only — never pushed into `sink` — so
/// a sudo password cannot appear in captured output.
async fn collect_until_sentinel(
    ch: &mut russh::Channel<Msg>,
    prefix: &str,
    deadline: Instant,
    sink: &mut ShellOutput,
    answer_rules: &[AnswerRule],
) -> SentinelEnd {
    if let Some((_, exit_code, cwd)) = find_sentinel(sink, prefix) {
        return SentinelEnd::Found { exit_code, cwd };
    }
    let mut fired = vec![false; answer_rules.len()];
    let end = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break SentinelEnd::TimedOut;
        }
        match tokio::time::timeout(remaining, ch.wait()).await {
            Ok(Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. })) => {
                if let Err(actual) = sink.append(&data) {
                    break SentinelEnd::OutputLimitExceeded { actual };
                }
                if let Some((_, exit_code, cwd)) = find_sentinel(sink, prefix) {
                    break SentinelEnd::Found { exit_code, cwd };
                }
                if timeout_at(deadline, maybe_inject_answers(ch, sink, answer_rules, &mut fired))
                    .await.is_err()
                {
                    break SentinelEnd::TimedOut;
                }
            }
            // The server closed the channel: the shell is gone, and no sentinel
            // will ever arrive. Reported distinctly so the caller does not treat
            // a dead link as a slow command.
            Ok(Some(ChannelMsg::Close)) | Ok(None) => break SentinelEnd::Closed,
            // Non-output messages (WindowAdjusted, Success, ...) — keep waiting.
            Ok(Some(_)) => continue,
            Err(_) => break SentinelEnd::TimedOut,
        }
    };
    match sink.finish() {
        Ok(()) => end,
        Err(actual) => SentinelEnd::OutputLimitExceeded { actual },
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
    let mut output = s.replace('\r', "");
    if output.ends_with('\n') {
        output.pop();
    }
    output
}


fn limit_error(error: crate::limits::LimitError) -> SshError {
    SshError::InvalidInput(error.to_string())
}

/// Drop cannot await or establish a remote exit proof. Try control requests
/// directly (they do not need a data window), without spawning detached work.
/// A full transport queue may reject these attempts; explicit close retries
/// within its budget and is the only path that reports a reaping proof.
fn retire_channel(channel: &russh::Channel<Msg>) {
    let _ = channel.signal(russh::Sig::INT).now_or_never();
    let _ = channel.signal(russh::Sig::TERM).now_or_never();
    let _ = channel.eof().now_or_never();
    let _ = channel.close().now_or_never();
}

impl Drop for RemoteShell {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.get_mut().unwrap().channel.take() {
            retire_channel(&channel);
        }
    }
}

fn change_directory_command(cwd: &str) -> String {
    // A relative operand must start with ./ so neither cd options / OLDPWD nor
    // inherited CDPATH can redirect it. Absolute paths keep their POSIX form.
    let cwd = if cwd.starts_with('/') { cwd.to_owned() } else { format!("./{cwd}") };
    format!("cd {}", shell_quote(&cwd))
}

/// Minimal single-quote shell quoting for the init `cd` path.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
#[path = "shell_output_tests.rs"]
mod output_tests;

#[cfg(test)]
#[path = "shell_protocol_tests.rs"]
mod protocol_tests;

#[cfg(test)]
#[path = "shell_directory_tests.rs"]
mod directory_tests;

#[cfg(test)]
#[path = "shell_budget_tests.rs"]
mod budget_tests;

#[cfg(test)]
#[path = "shell_retirement_tests.rs"]
mod retirement_tests;

#[cfg(test)]
mod tests {
    use super::{ShellOutput, limit_error};
    use crate::connection::SshError;
    use crate::limits::{
        MAX_SSH_COMMAND_BYTES, MAX_SSH_OUTPUT_BYTES, validate_command,
    };

    #[test]
    fn command_and_output_limits_are_typed_and_fail_closed() {
        let command = "x".repeat(MAX_SSH_COMMAND_BYTES + 1);
        let error = validate_command(&command)
            .map_err(limit_error)
            .expect_err("oversized command must be rejected");
        assert!(matches!(error, SshError::InvalidInput(_)));

        let mut output = ShellOutput::default();
        output.append(&vec![b'x'; MAX_SSH_OUTPUT_BYTES]).unwrap();
        assert_eq!(
            output.append(b"x"),
            Err(MAX_SSH_OUTPUT_BYTES + 1)
        );
        assert_eq!(output.len(), MAX_SSH_OUTPUT_BYTES);
    }
}
