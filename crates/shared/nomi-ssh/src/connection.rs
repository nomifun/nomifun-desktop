//! `SshConnection`: an authenticated russh client session plus a strict host-key
//! policy. This is the single entry point the pool uses; shells (`shell.rs`) and
//! SFTP (`fs.rs`) open channels off the authenticated handle.
//!
//! Host-key handling mirrors OpenSSH `StrictHostKeyChecking`:
//! - unknown host under `AcceptNew` → learn it into `~/.ssh/known_hosts`;
//! - unknown host under `Strict` → refuse with `HostKeyUnknown`;
//! - changed key (any policy) → refuse with `HostKeyChanged`, never auto-accept
//!   (the known_hosts file is shared with the operator's own `ssh`).
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, decode_secret_key};

use crate::credential::{Auth, SshCredential};

/// How the server's host key is validated on connect.
#[derive(Clone, Debug)]
pub enum HostKeyPolicy {
    /// Trust-on-first-use: an unknown key is learned into `known_hosts`.
    AcceptNew { known_hosts: PathBuf },
    /// Reject any key not already present in `known_hosts`.
    Strict { known_hosts: PathBuf },
}

impl HostKeyPolicy {
    fn known_hosts_path(&self) -> &PathBuf {
        match self {
            HostKeyPolicy::AcceptNew { known_hosts } | HostKeyPolicy::Strict { known_hosts } => {
                known_hosts
            }
        }
    }
    fn accepts_new(&self) -> bool {
        matches!(self, HostKeyPolicy::AcceptNew { .. })
    }
}

/// A failure opening or authenticating an SSH connection. Carries enough
/// structure for the UI to render the nine connection states without parsing
/// strings.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("cannot reach {0}")]
    Unreachable(String),
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    #[error("host key for {host} is unknown (fingerprint {fingerprint})")]
    HostKeyUnknown { host: String, fingerprint: String },
    #[error("host key for {host} changed (known_hosts line {line})")]
    HostKeyChanged { host: String, line: usize },
    #[error("ssh protocol error: {0}")]
    Protocol(String),
    /// The transport or shell channel went away mid-operation. Distinct from a
    /// slow command on purpose: there is no outcome to report, only a lost link,
    /// so the pool must redial rather than wait.
    #[error("ssh link disconnected: {0}")]
    Disconnected(String),
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Protocol(e.to_string())
    }
}

/// Why `check_server_key` rejected a key, captured out of the handler so the
/// caller can turn a generic connect failure into a precise `SshError`.
#[derive(Clone)]
enum RejectReason {
    Unknown { fingerprint: String },
    Changed { line: usize },
}

struct HandlerState {
    /// Set when `check_server_key` rejected; read after `connect` fails.
    reject: Option<RejectReason>,
    /// The server key fingerprint we observed, for display on success too.
    fingerprint: Option<String>,
}

pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    policy: HostKeyPolicy,
    state: Arc<Mutex<HandlerState>>,
}

impl client::Handler for ClientHandler {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.fingerprint(HashAlg::Sha256).to_string();
        self.state.lock().unwrap().fingerprint = Some(fingerprint.clone());

        let path = self.policy.known_hosts_path();
        match russh::keys::check_known_hosts_path(&self.host, self.port, server_public_key, path) {
            Ok(true) => Ok(true), // known and matches
            Ok(false) => {
                // Unknown host.
                if self.policy.accepts_new() {
                    russh::keys::known_hosts::learn_known_hosts_path(
                        &self.host,
                        self.port,
                        server_public_key,
                        path,
                    )
                    .map_err(|e| SshError::Protocol(e.to_string()))?;
                    Ok(true)
                } else {
                    self.state.lock().unwrap().reject =
                        Some(RejectReason::Unknown { fingerprint });
                    Ok(false)
                }
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                // Never auto-accept a changed key, even under AcceptNew.
                self.state.lock().unwrap().reject = Some(RejectReason::Changed { line });
                Ok(false)
            }
            Err(e) => Err(SshError::Protocol(e.to_string())),
        }
    }
}

/// An authenticated SSH session. Clone-cheap channel handle lives inside; open
/// shells / SFTP off it. Dropping it closes the transport.
pub struct SshConnection {
    handle: Handle<ClientHandler>,
    /// SHA256 fingerprint observed at connect, for UI display.
    pub fingerprint: Option<String>,
}

impl std::fmt::Debug for SshConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnection")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl SshConnection {
    /// Connect to `cred.host:cred.port`, validate the host key per `policy`, and
    /// authenticate. Phase 1 implements Password and PrivateKey; Certificate and
    /// Agent are wired in Phase 3.
    pub async fn connect(cred: &SshCredential, policy: HostKeyPolicy) -> Result<Self, SshError> {
        let config = Arc::new(client::Config::default());
        let state = Arc::new(Mutex::new(HandlerState {
            reject: None,
            fingerprint: None,
        }));
        let handler = ClientHandler {
            host: cred.host.clone(),
            port: cred.port,
            policy,
            state: Arc::clone(&state),
        };

        let mut handle = match client::connect(config, (cred.host.as_str(), cred.port), handler)
            .await
        {
            Ok(h) => h,
            Err(e) => {
                // A host-key rejection surfaces here as a generic handshake
                // error; translate it into the precise variant.
                let reason = state.lock().unwrap().reject.clone();
                return Err(match reason {
                    Some(RejectReason::Unknown { fingerprint }) => SshError::HostKeyUnknown {
                        host: cred.host.clone(),
                        fingerprint,
                    },
                    Some(RejectReason::Changed { line }) => SshError::HostKeyChanged {
                        host: cred.host.clone(),
                        line,
                    },
                    None => SshError::Unreachable(e.to_string()),
                });
            }
        };

        Self::authenticate(&mut handle, cred).await?;

        let fingerprint = state.lock().unwrap().fingerprint.clone();
        Ok(SshConnection { handle, fingerprint })
    }

    async fn authenticate(
        handle: &mut Handle<ClientHandler>,
        cred: &SshCredential,
    ) -> Result<(), SshError> {
        let ok = match &cred.auth {
            Auth::Password(pw) => handle
                .authenticate_password(cred.username.clone(), pw.as_str())
                .await?
                .success(),
            Auth::PrivateKey { pem, passphrase } => {
                let key = decode_secret_key(pem.as_str(), passphrase.as_deref().map(|s| s.as_str()))
                    .map_err(|e| SshError::AuthFailed(format!("private key: {e}")))?;
                handle
                    .authenticate_publickey(
                        cred.username.clone(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), None),
                    )
                    .await?
                    .success()
            }
            Auth::Certificate { .. } | Auth::Agent => {
                // Phase 3: certificate and ssh-agent authentication.
                return Err(SshError::AuthFailed(format!(
                    "{} auth is not yet supported (Phase 3)",
                    cred.auth.kind()
                )));
            }
        };
        if ok {
            Ok(())
        } else {
            Err(SshError::AuthFailed(format!(
                "{} authentication rejected by server",
                cred.auth.kind()
            )))
        }
    }

    /// Borrow the authenticated handle for opening channels (shell / SFTP).
    pub(crate) fn handle(&self) -> &Handle<ClientHandler> {
        &self.handle
    }

    /// Cheap in-process liveness bit: true once russh's session task has ended
    /// (TCP gone, server disconnected, transport error). No network I/O and no
    /// channel lock, so a supervisor can poll it on a timer while a long command
    /// still holds the shell. It cannot detect a silently black-holed link — for
    /// that the peer has to be pinged.
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Round-trip liveness probe: sends a keepalive and waits for the peer's
    /// reply, so a half-open TCP connection is detected instead of looking idle.
    pub async fn ping(&self) -> Result<(), SshError> {
        self.handle
            .send_ping()
            .await
            .map_err(|e| SshError::Disconnected(e.to_string()))
    }

    /// Send `SSH_MSG_DISCONNECT` so the server sees a deliberate close instead
    /// of a torn-down TCP connection.
    pub async fn disconnect(&self) -> Result<(), SshError> {
        self.handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await
            .map_err(|e| SshError::Disconnected(e.to_string()))
    }
}
