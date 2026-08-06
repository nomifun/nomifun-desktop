//! `SshBackend` implementations and the dialling primitives behind them.
//!
//! A [`SshConnectionHandle`] owns one live SSH connection: a persistent shell for
//! commands and cwd/env state, plus an SFTP session for file ops. The trait is
//! reached through the `nomifun-ai-agent` re-export — this crate has no
//! `nomi-agent`/`nomi-tools` dependency, only `nomi-ssh` (transport) and the seam.
//!
//! Connection identity and credentials are baked in at `connect`; the model never
//! sees them. This mirrors the Sink pattern (`nomifun-requirement`).
//!
//! There is exactly one `SshBackend` implementation, [`SshLinkBackend`], and it
//! resolves the pool's *current* handle on every call — so a reconnect underneath
//! is invisible to the tools the model is already holding, and no session can
//! exist outside the pool's accounting.
use std::sync::Arc;

use nomi_ssh::connection::{HostKeyPolicy, SshConnection, SshError};
use nomi_ssh::credential::{Auth, SshCredential};
use nomi_ssh::fs::RemoteFs;
use nomi_ssh::responder::AnswerRule;
use nomi_ssh::shell::{RemoteShell, ShellOutcome};
use nomifun_ai_agent::{RemoteCommandOutput, RemoteFileStat, SshBackend};
use zeroize::Zeroizing;

use crate::pool::{SshConnectionPool, SshLink};
use crate::service::{DecryptedCredential, SshServiceError};

/// Why a dial did not produce a usable link.
///
/// Coarser than [`SshError`] on purpose: the pool only ever has to decide
/// "retry, or stop and tell a human", and the operator only ever has to decide
/// "which of my settings is wrong". Both answers follow from the variant, never
/// from parsing the message.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SshDialError {
    /// The stored credential cannot be used at all: absent, of an auth kind this
    /// build does not implement, or the host row is gone.
    #[error("ssh credential is unusable: {0}")]
    Credential(String),
    #[error("ssh authentication failed: {0}")]
    Auth(String),
    #[error("ssh host key rejected: {0}")]
    HostKey(String),
    #[error("cannot reach the ssh host: {0}")]
    Unreachable(String),
    #[error("ssh protocol error: {0}")]
    Protocol(String),
    /// The pool is quiescing; it will not open new sockets it cannot close.
    #[error("the ssh connection pool is shutting down")]
    ShuttingDown,
}

impl SshDialError {
    /// Whether dialling again could plausibly work.
    ///
    /// Deliberately in lockstep with [`crate::state::is_retryable`]: the
    /// supervisor classifies a transport error while `acquire`'s caller
    /// classifies a dial error, and the two must never disagree about the same
    /// failure (pinned by `tests/dial_errors.rs`). Replaying a rejected
    /// credential only walks the account into a server-side lockout, and a host
    /// key that changed under us must not be re-accepted without a human.
    pub fn is_retryable(&self) -> bool {
        match self {
            SshDialError::Unreachable(_) | SshDialError::Protocol(_) => true,
            SshDialError::Credential(_)
            | SshDialError::Auth(_)
            | SshDialError::HostKey(_)
            | SshDialError::ShuttingDown => false,
        }
    }
}

impl From<SshError> for SshDialError {
    fn from(e: SshError) -> Self {
        match e {
            SshError::Unreachable(m) => SshDialError::Unreachable(m),
            // A link that died mid-flight is the same class of problem as one we
            // never reached: the host, not the credential.
            SshError::Disconnected(m) => SshDialError::Unreachable(m),
            SshError::AuthFailed(m) => SshDialError::Auth(m),
            SshError::HostKeyUnknown { host, fingerprint } => SshDialError::HostKey(format!(
                "host key for {host} is unknown (fingerprint {fingerprint})"
            )),
            SshError::HostKeyChanged { host, line } => SshDialError::HostKey(format!(
                "host key for {host} changed (known_hosts line {line})"
            )),
            SshError::Protocol(m) => SshDialError::Protocol(m),
        }
    }
}

impl From<SshServiceError> for SshDialError {
    /// Every host-book failure is a credential problem from the dialler's point
    /// of view: the row is missing, not ours, or will not decrypt. A retry fixes
    /// none of those.
    fn from(e: SshServiceError) -> Self {
        SshDialError::Credential(e.to_string())
    }
}

/// A live connection bound to one conversation: a persistent shell (cwd/env
/// survive across commands) and an SFTP session (file ops).
pub struct SshConnectionHandle {
    shell: Arc<RemoteShell>,
    fs: Arc<RemoteFs>,
    /// Kept alive so the transport is not dropped while shell/fs are in use, and
    /// so the channels can be reopened on it without a second handshake.
    conn: Arc<SshConnection>,
    /// SHA256 host fingerprint observed at connect (for status/display).
    pub fingerprint: Option<String>,
}

impl SshConnectionHandle {
    /// Dial, authenticate (host-key policy `AcceptNew` writes unknown keys to the
    /// operator's known_hosts), open a persistent shell rooted at `remote_cwd`
    /// (with the optional sudo answer rule installed), and open SFTP.
    pub async fn connect(
        cred: DecryptedCredential,
        known_hosts: std::path::PathBuf,
        remote_cwd: &str,
    ) -> Result<Self, SshDialError> {
        let ssh_cred = to_ssh_credential(&cred)?;
        let conn =
            SshConnection::connect(&ssh_cred, HostKeyPolicy::AcceptNew { known_hosts }).await?;
        let fingerprint = conn.fingerprint.clone();
        let conn = Arc::new(conn);

        let shell = conn
            .open_shell_with_rules(remote_cwd, sudo_rules(&cred))
            .await?;
        let fs = Arc::new(conn.open_sftp().await?);

        Ok(SshConnectionHandle {
            shell,
            fs,
            conn,
            fingerprint,
        })
    }

    pub(crate) fn shell(&self) -> &Arc<RemoteShell> {
        &self.shell
    }

    pub(crate) fn fs(&self) -> &Arc<RemoteFs> {
        &self.fs
    }

    pub(crate) fn conn(&self) -> &Arc<SshConnection> {
        &self.conn
    }

    /// Cheap in-process liveness bit. Takes no lock and does no I/O, so the
    /// supervisor can poll it while a long command still holds the shell. It
    /// cannot see a silently black-holed link — that needs a round trip.
    pub fn is_transport_closed(&self) -> bool {
        self.conn.is_closed()
    }

    /// Rebuild the shell and SFTP channels on the *same* authenticated transport.
    ///
    /// This is the recovery for a wedged shell: the socket, the session keys and
    /// the host-key decision are all still good, so redialling would cost a full
    /// handshake and would re-touch `known_hosts` for no reason.
    pub(crate) async fn reopen_channels(
        &self,
        cwd: &str,
        rules: Vec<AnswerRule>,
    ) -> Result<Self, SshDialError> {
        let shell = self.conn.open_shell_with_rules(cwd, rules).await?;
        let fs = Arc::new(self.conn.open_sftp().await?);
        Ok(SshConnectionHandle {
            shell,
            fs,
            conn: Arc::clone(&self.conn),
            fingerprint: self.fingerprint.clone(),
        })
    }
}

/// The host's sudo auto-answer rule, if it stored a sudo password. Rebuilt from a
/// freshly decrypted credential every time a shell is opened, so nothing above
/// the transport has to keep the password between dials.
pub(crate) fn sudo_rules(cred: &DecryptedCredential) -> Vec<AnswerRule> {
    match &cred.sudo_password {
        Some(pw) => vec![AnswerRule::sudo(Zeroizing::new(pw.as_str().to_string()))],
        None => Vec::new(),
    }
}

/// The `SshBackend` the pool hands out: it resolves the link's *current* handle
/// on every call, so a reconnect that swaps the transport underneath is invisible
/// to the tool objects the agent is already holding.
///
/// It is also where shell trouble is reported upwards: a lost transport or an
/// unrecoverable shell tells the pool, instead of being flattened into a string
/// the caller can only print.
pub struct SshLinkBackend {
    pool: SshConnectionPool,
    link: Arc<SshLink>,
}

impl SshLinkBackend {
    pub(crate) fn new(pool: SshConnectionPool, link: Arc<SshLink>) -> Self {
        Self { pool, link }
    }

    /// The live handle, or an explanation of what the link is doing instead. The
    /// message names the phase because "connection closed" is useless to a model
    /// that could usefully wait for a reconnect.
    async fn handle(&self) -> Result<Arc<SshConnectionHandle>, String> {
        self.link.current_handle().await.ok_or_else(|| {
            format!(
                "ssh link for this session is not connected ({:?})",
                self.link.state().phase()
            )
        })
    }

    /// Run one submission, keeping the pool's view of the link honest: a proven
    /// cwd is remembered for replay after a reconnect, a resync failure recycles
    /// the shell, and a lost transport starts the ladder now rather than at the
    /// next liveness tick.
    async fn run(&self, command: &str, timeout_ms: u64) -> Result<RemoteCommandOutput, String> {
        let handle = self.handle().await?;
        match run_with_budget(handle.shell(), command, timeout_ms).await {
            Ok(outcome) => {
                if outcome.timed_out && outcome.cwd.is_empty() {
                    // `RemoteShell::run` only withholds the cwd on a timeout it
                    // could not resynchronize from: the transport is fine, the
                    // shell is not, so recycling the channel is the fix — not a
                    // redial, and certainly not swallowing it.
                    self.pool
                        .recycle_shell(
                            &self.link,
                            "remote shell could not be resynchronized after a timeout",
                        )
                        .await;
                } else if !outcome.cwd.is_empty() {
                    self.link.remember_cwd(&outcome.cwd);
                }
                Ok(remote_output(outcome))
            }
            Err(e @ SshError::Disconnected(_)) => {
                let detail = e.to_string();
                self.pool.note_transport_loss(&self.link, &detail).await;
                Err(detail)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

#[async_trait::async_trait]
impl SshBackend for SshLinkBackend {
    async fn run_command(
        &self,
        command: &str,
        timeout_ms: u64,
    ) -> Result<RemoteCommandOutput, String> {
        self.run(command, timeout_ms).await
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.handle()
            .await?
            .fs()
            .read_file(path)
            .await
            .map_err(|e| e.to_string())
    }

    async fn write_file(&self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.handle()
            .await?
            .fs()
            .write_file_atomic(path, &bytes)
            .await
            .map_err(|e| e.to_string())
    }

    async fn grep(&self, pattern: &str, path: &str) -> Result<String, String> {
        let out = self
            .run(&grep_command(pattern, path), GREP_TIMEOUT_MS)
            .await?;
        Ok(out.stdout)
    }

    async fn list_files(&self, glob: &str) -> Result<Vec<String>, String> {
        let out = self.run(&list_command(glob), LIST_TIMEOUT_MS).await?;
        Ok(list_lines(&out.stdout))
    }

    async fn stat(&self, path: &str) -> Result<RemoteFileStat, String> {
        let s = self
            .handle()
            .await?
            .fs()
            .stat(path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(RemoteFileStat {
            size: s.size,
            is_dir: s.is_dir,
        })
    }
}

/// Budget for a `grep` submission — the tool has no timeout parameter, so this
/// is the ceiling on a recursive search over a big tree.
const GREP_TIMEOUT_MS: u64 = 30_000;
/// Budget for a `ls -1d` submission.
const LIST_TIMEOUT_MS: u64 = 15_000;

async fn run_with_budget(
    shell: &Arc<RemoteShell>,
    command: &str,
    timeout_ms: u64,
) -> Result<ShellOutcome, SshError> {
    shell
        .run(command, std::time::Duration::from_millis(timeout_ms))
        .await
}

fn remote_output(outcome: ShellOutcome) -> RemoteCommandOutput {
    RemoteCommandOutput {
        stdout: outcome.output,
        exit_code: outcome.exit_code,
        timed_out: outcome.timed_out,
    }
}

/// ripgrep if present, else `grep -rn`. Pattern and path are single-quoted to
/// keep them off the shell's parsing surface; `--color=never` keeps ANSI escapes
/// out of the captured output.
fn grep_command(pattern: &str, path: &str) -> String {
    format!(
        "rg --color=never -n -- {p} {d} 2>/dev/null || grep --color=never -rn -- {p} {d} 2>/dev/null || true",
        p = sh_quote(pattern),
        d = sh_quote(path),
    )
}

/// Rely on the remote shell's glob expansion; unmatched globs yield nothing
/// (nullglob-ish via the `2>/dev/null` + filtering).
fn list_command(glob: &str) -> String {
    format!("ls -1d {} 2>/dev/null || true", sh_quote_glob(glob))
}

fn list_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Map a decrypted credential bundle onto the transport's `SshCredential`.
fn to_ssh_credential(cred: &DecryptedCredential) -> Result<SshCredential, SshDialError> {
    let auth = match cred.auth_type.as_str() {
        "password" => Auth::Password(clone_secret(
            cred.password.as_ref(),
            "password auth selected but no password stored",
        )?),
        "key" => Auth::PrivateKey {
            pem: clone_secret(
                cred.private_key.as_ref(),
                "key auth selected but no private key stored",
            )?,
            passphrase: cred
                .passphrase
                .as_ref()
                .map(|p| Zeroizing::new(p.as_str().to_string())),
        },
        "certificate" => Auth::Certificate {
            key_pem: clone_secret(
                cred.private_key.as_ref(),
                "certificate auth selected but no private key stored",
            )?,
            // The certificate is public material (it is what gets shown to the
            // server), so unlike the key it needs no `Zeroizing`.
            cert: cred
                .certificate
                .as_ref()
                .map(|c| c.as_str().to_string())
                .ok_or_else(|| {
                    SshDialError::Credential(
                        "certificate auth selected but no certificate stored".to_string(),
                    )
                })?,
            passphrase: cred
                .passphrase
                .as_ref()
                .map(|p| Zeroizing::new(p.as_str().to_string())),
        },
        // Nothing is stored for agent auth: the keys stay in the operator's
        // ssh-agent, which the transport finds through this process's
        // `SSH_AUTH_SOCK`.
        "agent" => Auth::Agent { socket: None },
        other => {
            return Err(SshDialError::Credential(format!(
                "unknown auth_type {other:?}"
            )));
        }
    };
    Ok(SshCredential {
        host: cred.host.clone(),
        port: cred.port,
        username: cred.username.clone(),
        auth,
    })
}

fn clone_secret(
    v: Option<&Zeroizing<String>>,
    missing: &str,
) -> Result<Zeroizing<String>, SshDialError> {
    v.map(|z| Zeroizing::new(z.as_str().to_string()))
        .ok_or_else(|| SshDialError::Credential(missing.to_string()))
}

/// Single-quote a value for POSIX sh, escaping embedded single quotes.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Glob patterns must NOT be single-quoted (that would disable expansion), but
/// we still guard against shell metacharacters beyond glob wildcards by
/// rejecting quotes/backticks/`$`/`;`. Anything suspicious is single-quoted
/// (treated literally) instead.
fn sh_quote_glob(s: &str) -> String {
    if s.contains(['\'', '`', '$', ';', '|', '&', '\n', '"']) {
        sh_quote(s)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_error_retryability_agrees_with_the_state_classifier() {
        for err in [
            SshError::Unreachable("refused".into()),
            SshError::Disconnected("eof".into()),
            SshError::Protocol("kex".into()),
            SshError::AuthFailed("rejected".into()),
            SshError::HostKeyUnknown {
                host: "h".into(),
                fingerprint: "f".into(),
            },
            SshError::HostKeyChanged {
                host: "h".into(),
                line: 1,
            },
        ] {
            let expected = crate::state::is_retryable(&err);
            let mapped = SshDialError::from(err);
            assert_eq!(
                mapped.is_retryable(),
                expected,
                "{mapped:?} disagrees with the state classifier"
            );
        }
    }

    #[test]
    fn a_shutting_down_pool_is_not_retryable() {
        assert!(!SshDialError::ShuttingDown.is_retryable());
        assert!(!SshDialError::Credential("no password".into()).is_retryable());
    }

    #[test]
    fn glob_and_pattern_quoting_is_unchanged() {
        // Pinned because these strings are what actually runs on the operator's
        // host; a "harmless" tidy-up here is a command-injection change.
        assert_eq!(sh_quote("a'b"), r#"'a'\''b'"#);
        assert_eq!(sh_quote_glob("*.rs"), "*.rs");
        assert_eq!(sh_quote_glob("$(id)"), r#"'$(id)'"#);
        assert!(grep_command("x", "/srv").starts_with("rg --color=never -n -- 'x' '/srv'"));
        assert_eq!(list_command("*.rs"), "ls -1d *.rs 2>/dev/null || true");
    }
}
