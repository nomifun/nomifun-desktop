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
