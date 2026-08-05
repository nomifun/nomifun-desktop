//! Credential material for an SSH connection. All secret fields are held in
//! `Zeroizing` so they are wiped on drop, and elided from `Debug` so they never
//! leak into logs, panics, or error chains.
use zeroize::Zeroizing;

/// Everything needed to open and authenticate one SSH connection. Secret
/// material lives in `Auth`; the non-secret coordinates (host/port/username)
/// are plain fields so they remain visible in `Debug` for diagnostics.
pub struct SshCredential {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: Auth,
}

/// The authentication method plus its secret material. `Password`,
/// `PrivateKey` and `Certificate` carry `Zeroizing` secrets; `Agent` defers to
/// the operator's running ssh-agent and holds nothing.
pub enum Auth {
    Password(Zeroizing<String>),
    PrivateKey {
        /// PEM/OpenSSH-format private key body.
        pem: Zeroizing<String>,
        passphrase: Option<Zeroizing<String>>,
    },
    Certificate {
        /// PEM/OpenSSH-format private key body the certificate was issued for.
        key_pem: Zeroizing<String>,
        /// The OpenSSH certificate (`*-cert.pub`) contents — not secret.
        cert: String,
        passphrase: Option<Zeroizing<String>>,
    },
    Agent,
}

impl Auth {
    /// Stable, non-secret label for the method — safe to log and to surface in
    /// the UI. Never derived from secret bytes.
    pub fn kind(&self) -> &'static str {
        match self {
            Auth::Password(_) => "password",
            Auth::PrivateKey { .. } => "key",
            Auth::Certificate { .. } => "certificate",
            Auth::Agent => "agent",
        }
    }
}

impl std::fmt::Debug for SshCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshCredential")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &format_args!("{}(<redacted>)", self.auth.kind()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_secret_material() {
        let cred = SshCredential {
            host: "example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: Auth::Password(Zeroizing::new("hunter2_supersecret".into())),
        };
        let rendered = format!("{cred:?}");
        assert!(
            !rendered.contains("hunter2_supersecret"),
            "secret leaked in Debug: {rendered}"
        );
        assert!(
            rendered.contains("example.com"),
            "non-secret host should still be visible: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "secret should render as <redacted>: {rendered}"
        );
        assert!(
            rendered.contains("password"),
            "auth kind label should be visible: {rendered}"
        );
    }

    #[test]
    fn debug_elides_private_key_body() {
        let cred = SshCredential {
            host: "10.0.0.1".into(),
            port: 22,
            username: "ci".into(),
            auth: Auth::PrivateKey {
                pem: Zeroizing::new("-----BEGIN OPENSSH PRIVATE KEY-----\nSECRETBODY\n".into()),
                passphrase: Some(Zeroizing::new("passphrase_secret".into())),
            },
        };
        let rendered = format!("{cred:?}");
        assert!(!rendered.contains("SECRETBODY"), "key body leaked: {rendered}");
        assert!(!rendered.contains("passphrase_secret"), "passphrase leaked: {rendered}");
        assert!(rendered.contains("key(<redacted>)"), "got: {rendered}");
    }

    #[test]
    fn kind_labels_are_stable() {
        assert_eq!(Auth::Agent.kind(), "agent");
        assert_eq!(
            Auth::Certificate {
                key_pem: Zeroizing::new(String::new()),
                cert: String::new(),
                passphrase: None,
            }
            .kind(),
            "certificate"
        );
    }
}
