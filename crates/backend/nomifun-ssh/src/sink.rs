//! `SshBackendSink`: the backend implementation of the agent's `SshBackend`
//! seam. It owns a live SSH connection (a persistent shell for commands and cwd/
//! env state, plus an SFTP session for file ops) and reaches the trait through
//! the `nomifun-ai-agent` re-export — this crate has no `nomi-agent`/`nomi-tools`
//! dependency, only `nomi-ssh` (transport) and the seam.
//!
//! Connection identity and credentials are baked in at `connect`; the model
//! never sees them. This mirrors the Sink pattern (`nomifun-requirement`).
use std::sync::Arc;

use nomi_ssh::connection::{HostKeyPolicy, SshConnection};
use nomi_ssh::credential::{Auth, SshCredential};
use nomi_ssh::fs::RemoteFs;
use nomi_ssh::responder::AnswerRule;
use nomi_ssh::shell::RemoteShell;
use nomifun_ai_agent::{RemoteCommandOutput, RemoteFileStat, SshBackend};
use zeroize::Zeroizing;

use crate::service::DecryptedCredential;

/// A live connection bound to one conversation: a persistent shell (cwd/env
/// survive across commands) and an SFTP session (file ops).
pub struct SshConnectionHandle {
    shell: Arc<RemoteShell>,
    fs: Arc<RemoteFs>,
    /// Kept alive so the transport is not dropped while shell/fs are in use.
    _conn: Arc<SshConnection>,
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
    ) -> Result<Self, String> {
        let ssh_cred = to_ssh_credential(&cred)?;
        let conn = SshConnection::connect(
            &ssh_cred,
            HostKeyPolicy::AcceptNew { known_hosts },
        )
        .await
        .map_err(|e| e.to_string())?;
        let fingerprint = conn.fingerprint.clone();
        let conn = Arc::new(conn);

        let rules = match &cred.sudo_password {
            Some(pw) => vec![AnswerRule::sudo(Zeroizing::new(pw.as_str().to_string()))],
            None => Vec::new(),
        };
        let shell = conn
            .open_shell_with_rules(remote_cwd, rules)
            .await
            .map_err(|e| e.to_string())?;
        let fs = Arc::new(conn.open_sftp().await.map_err(|e| e.to_string())?);

        Ok(SshConnectionHandle {
            shell,
            fs,
            _conn: conn,
            fingerprint,
        })
    }
}

/// The `SshBackend` implementation handed to the agent's remote tools.
pub struct SshBackendSink {
    handle: Arc<SshConnectionHandle>,
}

impl SshBackendSink {
    pub fn new(handle: Arc<SshConnectionHandle>) -> Self {
        Self { handle }
    }

    /// Erase to the seam trait object the agent bootstrap expects.
    pub fn into_arc(handle: Arc<SshConnectionHandle>) -> Arc<dyn SshBackend> {
        Arc::new(Self::new(handle))
    }
}

#[async_trait::async_trait]
impl SshBackend for SshBackendSink {
    async fn run_command(
        &self,
        command: &str,
        timeout_ms: u64,
    ) -> Result<RemoteCommandOutput, String> {
        let outcome = self
            .handle
            .shell
            .run(command, std::time::Duration::from_millis(timeout_ms))
            .await
            .map_err(|e| e.to_string())?;
        Ok(RemoteCommandOutput {
            stdout: outcome.output,
            exit_code: outcome.exit_code,
            timed_out: outcome.timed_out,
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.handle.fs.read_file(path).await.map_err(|e| e.to_string())
    }

    async fn write_file(&self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.handle
            .fs
            .write_file_atomic(path, &bytes)
            .await
            .map_err(|e| e.to_string())
    }

    async fn grep(&self, pattern: &str, path: &str) -> Result<String, String> {
        // ripgrep if present, else grep -rn. Pattern and path are single-quoted
        // to keep them off the shell's parsing surface; `--color=never` keeps
        // ANSI escapes out of the captured output.
        let cmd = format!(
            "rg --color=never -n -- {p} {d} 2>/dev/null || grep --color=never -rn -- {p} {d} 2>/dev/null || true",
            p = sh_quote(pattern),
            d = sh_quote(path),
        );
        let out = self
            .handle
            .shell
            .run(&cmd, std::time::Duration::from_secs(30))
            .await
            .map_err(|e| e.to_string())?;
        Ok(out.output)
    }

    async fn list_files(&self, glob: &str) -> Result<Vec<String>, String> {
        // Rely on the remote shell's glob expansion; unmatched globs yield
        // nothing (nullglob-ish via the `2>/dev/null` + filtering).
        let cmd = format!("ls -1d {} 2>/dev/null || true", sh_quote_glob(glob));
        let out = self
            .handle
            .shell
            .run(&cmd, std::time::Duration::from_secs(15))
            .await
            .map_err(|e| e.to_string())?;
        Ok(out
            .output
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect())
    }

    async fn stat(&self, path: &str) -> Result<RemoteFileStat, String> {
        let s = self.handle.fs.stat(path).await.map_err(|e| e.to_string())?;
        Ok(RemoteFileStat {
            size: s.size,
            is_dir: s.is_dir,
        })
    }
}

/// Map a decrypted credential bundle onto the transport's `SshCredential`.
fn to_ssh_credential(cred: &DecryptedCredential) -> Result<SshCredential, String> {
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
            passphrase: cred.passphrase.as_ref().map(|p| Zeroizing::new(p.as_str().to_string())),
        },
        "certificate" | "agent" => {
            return Err(format!("{} auth is not supported in Phase 1", cred.auth_type));
        }
        other => return Err(format!("unknown auth_type {other:?}")),
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
) -> Result<Zeroizing<String>, String> {
    v.map(|z| Zeroizing::new(z.as_str().to_string()))
        .ok_or_else(|| missing.to_string())
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
