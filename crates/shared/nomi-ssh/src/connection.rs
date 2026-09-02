//! `SshConnection`: an authenticated russh client session plus a strict host-key
//! policy. This is the single entry point the pool uses; shells (`shell.rs`) and
//! SFTP (`fs.rs`) open channels off the authenticated handle.
//!
//! Host-key handling mirrors OpenSSH `StrictHostKeyChecking`:
//! - unknown host under `AcceptNew` → learn it into `~/.ssh/known_hosts`;
//! - unknown host under `Strict` → refuse with `HostKeyUnknown`;
//! - changed key (any policy) → refuse with `HostKeyChanged`, never auto-accept
//!   (the known_hosts file is shared with the operator's own `ssh`).
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use russh::client::{self, Handle};
#[cfg(unix)]
use russh::keys::agent::AgentIdentity;
#[cfg(unix)]
use russh::keys::agent::client::AgentClient;
use russh::keys::ssh_key::certificate::CertType;
#[cfg(unix)]
use russh::keys::{Algorithm, PublicKey};
use russh::keys::{
    Certificate, HashAlg, PrivateKey, PrivateKeyWithHashAlg, decode_secret_key,
};

use crate::credential::{Auth, SshCredential};
use crate::limits::{
    MAX_SSH_AGENT_SOCKET_BYTES, MAX_SSH_HOST_BYTES, MAX_SSH_USERNAME_BYTES, SSH_CONNECT_TIMEOUT,
    SSH_OPERATION_TIMEOUT, validate_credential, validate_endpoint_component,
};

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
    #[error("invalid SSH input: {0}")]
    InvalidInput(String),
    #[error("SSH operation timed out: {0}")]
    TimedOut(String),
    /// The transport or shell channel went away mid-operation. Distinct from a
    /// slow command on purpose: there is no outcome to report, only a lost link,
    /// so the pool must redial rather than wait.
    #[error("ssh link disconnected: {0}")]
    Disconnected(String),
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        match e {
            russh::Error::Disconnect
            | russh::Error::HUP
            | russh::Error::SendError => SshError::Disconnected(e.to_string()),
            russh::Error::ConnectionTimeout
            | russh::Error::KeepaliveTimeout
            | russh::Error::InactivityTimeout => SshError::TimedOut(e.to_string()),
            other => SshError::Protocol(other.to_string()),
        }
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
    /// authenticate with whichever of the four methods `cred.auth` names.
    pub async fn connect(cred: &SshCredential, policy: HostKeyPolicy) -> Result<Self, SshError> {
        validate_credential_input(cred)?;
        match tokio::time::timeout(SSH_CONNECT_TIMEOUT, Self::connect_inner(cred, policy)).await {
            Ok(result) => result,
            Err(_) => Err(SshError::TimedOut(format!(
                "SSH connect/authentication exceeded {}ms",
                SSH_CONNECT_TIMEOUT.as_millis()
            ))),
        }
    }

    async fn connect_inner(
        cred: &SshCredential,
        policy: HostKeyPolicy,
    ) -> Result<Self, SshError> {
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
            Auth::Certificate {
                key_pem,
                cert,
                passphrase,
            } => {
                let (key, cert) = certificate_material(
                    key_pem.as_str(),
                    cert,
                    passphrase.as_deref().map(|s| s.as_str()),
                )?;
                if handle
                    .authenticate_openssh_cert(cred.username.clone(), key, cert.clone())
                    .await?
                    .success()
                {
                    return Ok(());
                }
                // The server never says *why* it refused a certificate, but the
                // certificate itself distinguishes the causes that matter.
                return Err(SshError::AuthFailed(certificate_rejection(
                    &cert,
                    &cred.username,
                )));
            }
            Auth::Agent { socket } => {
                return authenticate_with_agent(handle, cred, socket.as_deref()).await;
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
        match tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.handle.send_ping()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(SshError::from(e)),
            Err(_) => Err(SshError::TimedOut(format!(
                "SSH keepalive exceeded {}ms",
                SSH_OPERATION_TIMEOUT.as_millis()
            ))),
        }
    }

    /// Send `SSH_MSG_DISCONNECT` so the server sees a deliberate close instead
    /// of a torn-down TCP connection.
    pub async fn disconnect(&self) -> Result<(), SshError> {
        match tokio::time::timeout(
            SSH_OPERATION_TIMEOUT,
            self.handle
                .disconnect(russh::Disconnect::ByApplication, "", "en"),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(SshError::from(e)),
            Err(_) => Err(SshError::TimedOut(format!(
                "SSH disconnect exceeded {}ms",
                SSH_OPERATION_TIMEOUT.as_millis()
            ))),
        }
    }
}

fn validate_credential_input(cred: &SshCredential) -> Result<(), SshError> {
    if cred.port == 0 {
        return Err(SshError::InvalidInput(
            "SSH port must be between 1 and 65535".to_string(),
        ));
    }
    validate_endpoint_component("SSH host", &cred.host, MAX_SSH_HOST_BYTES)
        .map_err(|e| SshError::InvalidInput(e.to_string()))?;
    validate_endpoint_component("SSH username", &cred.username, MAX_SSH_USERNAME_BYTES)
        .map_err(|e| SshError::InvalidInput(e.to_string()))?;

    match &cred.auth {
        Auth::Password(password) => {
            validate_credential("SSH password", password.as_str())
                .map_err(|e| SshError::InvalidInput(e.to_string()))?;
        }
        Auth::PrivateKey { pem, passphrase } => {
            validate_credential("SSH private key", pem.as_str())
                .map_err(|e| SshError::InvalidInput(e.to_string()))?;
            if let Some(passphrase) = passphrase {
                validate_credential("SSH key passphrase", passphrase.as_str())
                    .map_err(|e| SshError::InvalidInput(e.to_string()))?;
            }
        }
        Auth::Certificate {
            key_pem,
            cert,
            passphrase,
        } => {
            validate_credential("SSH certificate key", key_pem.as_str())
                .map_err(|e| SshError::InvalidInput(e.to_string()))?;
            validate_credential("SSH certificate", cert)
                .map_err(|e| SshError::InvalidInput(e.to_string()))?;
            if let Some(passphrase) = passphrase {
                validate_credential("SSH certificate passphrase", passphrase.as_str())
                    .map_err(|e| SshError::InvalidInput(e.to_string()))?;
            }
        }
        Auth::Agent { socket } => {
            if let Some(socket) = socket {
                let socket_len = socket.as_os_str().len();
                if socket_len > MAX_SSH_AGENT_SOCKET_BYTES {
                    return Err(SshError::InvalidInput(format!(
                        "SSH agent socket path is {socket_len} bytes; maximum is {}",
                        MAX_SSH_AGENT_SOCKET_BYTES
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Decode a private key plus the certificate issued for it, refusing up front the
/// two paste errors we can detect without asking the server: a body that is not a
/// user certificate, and a certificate belonging to some other key. Both would
/// otherwise come back as an indistinguishable "rejected by server".
fn certificate_material(
    key_pem: &str,
    cert_body: &str,
    passphrase: Option<&str>,
) -> Result<(Arc<PrivateKey>, Certificate), SshError> {
    let key = decode_secret_key(key_pem, passphrase)
        .map_err(|e| SshError::AuthFailed(format!("private key: {e}")))?;
    // `from_openssh` only tolerates trailing whitespace; a pasted cert routinely
    // has leading whitespace too.
    let cert = Certificate::from_openssh(cert_body.trim()).map_err(|e| {
        SshError::AuthFailed(format!(
            "certificate: {e} — expected the contents of a `*-cert.pub` file"
        ))
    })?;
    if cert.cert_type() != CertType::User {
        return Err(SshError::AuthFailed(
            "certificate: this is a host certificate, not a user certificate".to_string(),
        ));
    }
    if cert.public_key() != key.public_key().key_data() {
        return Err(SshError::AuthFailed(
            "certificate: it was issued for a different public key and does not match the \
             private key supplied"
                .to_string(),
        ));
    }
    Ok((Arc::new(key), cert))
}

/// Why the server most likely refused a certificate we consider well-formed.
///
/// SSH gives the client no reason code for an auth failure, so a bare
/// "authentication failed" leaves the operator guessing between three unrelated
/// fixes. The certificate itself settles the first two, and the third is what is
/// left over.
fn certificate_rejection(cert: &Certificate, username: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now > cert.valid_before() {
        return format!(
            "certificate expired {} ago; ask your CA to re-issue it",
            duration_words(now - cert.valid_before())
        );
    }
    if now < cert.valid_after() {
        return format!(
            "certificate is not valid for another {}; check this machine's clock",
            duration_words(cert.valid_after() - now)
        );
    }
    let principals = cert.valid_principals();
    if !principals.is_empty() && !principals.iter().any(|p| p == username) {
        return format!(
            "certificate is valid for principals [{}] but you are connecting as {username:?}",
            principals.join(", ")
        );
    }
    format!(
        "server rejected the certificate; its signing CA (fingerprint {}) is most likely absent \
         from the server's TrustedUserCAKeys",
        cert.signature_key().fingerprint(HashAlg::Sha256)
    )
}

/// Coarse single-unit rendering, so an expiry message reads "21m" instead of a
/// unix timestamp the operator has to convert by hand.
fn duration_words(secs: u64) -> String {
    match secs {
        s if s < 120 => format!("{s}s"),
        s if s < 7_200 => format!("{}m", s / 60),
        s if s < 172_800 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

/// Authenticate through a running ssh-agent, offering each identity it holds
/// until one is accepted. Signing stays inside the agent — no private key ever
/// enters this process.
///
/// Unix only: russh reaches an agent over a Unix-domain socket
/// (`AgentClient::connect_uds` does not exist on Windows, where agents speak a
/// named pipe or Pageant instead). See the `not(unix)` twin below.
#[cfg(unix)]
async fn authenticate_with_agent(
    handle: &mut Handle<ClientHandler>,
    cred: &SshCredential,
    socket: Option<&Path>,
) -> Result<(), SshError> {
    let path = match socket {
        Some(p) => p.to_path_buf(),
        // Only ever *read*: this process's SSH_AUTH_SOCK is never set or changed,
        // so the operator's own agent session is untouched either way.
        None => std::env::var_os("SSH_AUTH_SOCK")
            .map(PathBuf::from)
            .ok_or_else(|| {
                SshError::AuthFailed(
                    "no ssh-agent found in this process's environment (SSH_AUTH_SOCK is unset); \
                     a desktop app started from a launcher usually does not inherit it — use key \
                     or certificate auth for this host instead"
                        .to_string(),
                )
            })?,
    };
    let mut agent = AgentClient::connect_uds(&path).await.map_err(|e| {
        SshError::AuthFailed(format!(
            "cannot reach the ssh-agent at {}: {e}",
            path.display()
        ))
    })?;
    let identities = agent.request_identities().await.map_err(|e| {
        SshError::AuthFailed(format!(
            "the ssh-agent at {} would not list its identities: {e}",
            path.display()
        ))
    })?;
    if identities.is_empty() {
        return Err(SshError::AuthFailed(format!(
            "the ssh-agent at {} holds no identities (check `ssh-add -l`)",
            path.display()
        )));
    }

    let offered = identities.len();
    for identity in &identities {
        let hash_alg = rsa_hash_alg(handle, &identity.public_key()).await;
        let attempt = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(
                        cred.username.clone(),
                        key.clone(),
                        hash_alg,
                        &mut agent,
                    )
                    .await
            }
            // An agent can hold a certificate as well as the bare key; offering
            // it as a plain public key would fail against a cert-only server.
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(
                        cred.username.clone(),
                        certificate.clone(),
                        hash_alg,
                        &mut agent,
                    )
                    .await
            }
        };
        match attempt {
            Ok(result) if result.success() => return Ok(()),
            Ok(_) => continue,
            // The agent itself failed (locked, key removed mid-flight, socket
            // gone). Trying the next identity would hit the same wall.
            Err(e) => {
                return Err(SshError::AuthFailed(format!(
                    "the ssh-agent at {} refused to sign: {e}",
                    path.display()
                )));
            }
        }
    }
    Err(SshError::AuthFailed(format!(
        "the server accepted none of the {offered} {} offered by the ssh-agent at {}",
        if offered == 1 { "identity" } else { "identities" },
        path.display()
    )))
}

/// RSA identities need an explicit signature hash: servers have been dropping the
/// SHA-1 `ssh-rsa` algorithm, so ask this server which it will take. Every other
/// algorithm carries its hash choice in the key type itself.
#[cfg(unix)]
async fn rsa_hash_alg(handle: &Handle<ClientHandler>, key: &PublicKey) -> Option<HashAlg> {
    match key.algorithm() {
        Algorithm::Rsa { .. } => handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten(),
        _ => None,
    }
}

/// Windows (and anything else without Unix sockets) cannot reach an agent through
/// russh, so say that instead of failing as if the server had refused us.
#[cfg(not(unix))]
async fn authenticate_with_agent(
    _handle: &mut Handle<ClientHandler>,
    _cred: &SshCredential,
    _socket: Option<&Path>,
) -> Result<(), SshError> {
    Err(SshError::AuthFailed(
        "ssh-agent auth is not available on this platform — russh reaches agents over a \
         Unix-domain socket only; use key or certificate auth for this host"
            .to_string(),
    ))
}
