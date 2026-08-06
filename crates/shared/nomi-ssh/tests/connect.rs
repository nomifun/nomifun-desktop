//! Connection + auth + host-key policy against a real throwaway sshd.
mod support;

use nomi_ssh::connection::{HostKeyPolicy, SshConnection, SshError};
use nomi_ssh::credential::{Auth, SshCredential};

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_authenticates_against_real_sshd() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd in this environment (honest skip, not a pass-fake)");
        return;
    };
    let cred = SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::PrivateKey {
            pem: sshd.client_key_pem(),
            passphrase: None,
        },
    };
    let conn = SshConnection::connect(
        &cred,
        HostKeyPolicy::AcceptNew {
            known_hosts: sshd.known_hosts_path(),
        },
    )
    .await
    .expect("connect");
    assert!(
        conn.fingerprint.as_deref().unwrap_or("").starts_with("SHA256:"),
        "should observe a SHA256 host fingerprint, got {:?}",
        conn.fingerprint
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accept_new_learns_key_then_strict_accepts_it() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let cred = SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::PrivateKey {
            pem: sshd.client_key_pem(),
            passphrase: None,
        },
    };
    // First connect under AcceptNew learns the key.
    SshConnection::connect(
        &cred,
        HostKeyPolicy::AcceptNew {
            known_hosts: sshd.known_hosts_path(),
        },
    )
    .await
    .expect("accept-new connect");
    // Now Strict must accept the learned key.
    SshConnection::connect(
        &cred,
        HostKeyPolicy::Strict {
            known_hosts: sshd.known_hosts_path(),
        },
    )
    .await
    .expect("strict connect against learned key");
}

#[tokio::test(flavor = "multi_thread")]
async fn strict_rejects_unknown_host() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let cred = SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::PrivateKey {
            pem: sshd.client_key_pem(),
            passphrase: None,
        },
    };
    // Empty known_hosts + Strict → unknown host must be refused.
    let err = SshConnection::connect(
        &cred,
        HostKeyPolicy::Strict {
            known_hosts: sshd.known_hosts_path(),
        },
    )
    .await
    .expect_err("strict must reject unknown host");
    assert!(
        matches!(err, SshError::HostKeyUnknown { .. }),
        "expected HostKeyUnknown, got {err:?}"
    );
}

/// The one host-key invariant that matters in production, where the only policy
/// ever used is `AcceptNew`: a key that *changed* is refused, and known_hosts is
/// left byte-for-byte alone. `AcceptNew` learns unknown keys by appending to the
/// very file the operator's own `ssh` reads, so silently re-learning a changed
/// key would both wave through a man-in-the-middle and destroy the operator's
/// record of the real key.
#[tokio::test(flavor = "multi_thread")]
async fn accept_new_refuses_a_changed_key_without_touching_known_hosts() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let known_hosts = sshd.known_hosts_path();
    // Record the *wrong* key for this host:port — the fixture's spare key, which
    // is the same algorithm as the host key (russh only reports KeyChanged when
    // the algorithms match) but a different key.
    let wrong_key = std::fs::read_to_string(sshd.spare_key_path().with_extension("pub"))
        .expect("spare public key");
    std::fs::write(
        &known_hosts,
        format!("[127.0.0.1]:{} {wrong_key}", sshd.port()),
    )
    .expect("seed known_hosts");
    let before = std::fs::read_to_string(&known_hosts).expect("read known_hosts");

    let cred = SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::PrivateKey {
            pem: sshd.client_key_pem(),
            passphrase: None,
        },
    };
    let err = SshConnection::connect(
        &cred,
        HostKeyPolicy::AcceptNew {
            known_hosts: known_hosts.clone(),
        },
    )
    .await
    .expect_err("a changed host key must be refused even under AcceptNew");
    assert!(
        matches!(err, SshError::HostKeyChanged { .. }),
        "expected HostKeyChanged, got {err:?}"
    );

    let after = std::fs::read_to_string(&known_hosts).expect("read known_hosts");
    assert_eq!(
        after, before,
        "the refusal must not rewrite the operator's known_hosts"
    );
}
