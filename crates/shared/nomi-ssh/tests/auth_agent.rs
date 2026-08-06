//! ssh-agent authentication against a real throwaway sshd, driven by a **private**
//! agent this test starts on its own socket.
//!
//! The operator's own agent is deliberately unreachable from here: every case
//! passes an explicit socket path, and nothing in this file reads or writes
//! `SSH_AUTH_SOCK`.
mod support;

use std::path::PathBuf;

use nomi_ssh::connection::{HostKeyPolicy, SshConnection, SshError};
use nomi_ssh::credential::{Auth, SshCredential};

fn agent_cred(sshd: &support::sshd::TestSshd, socket: Option<PathBuf>) -> SshCredential {
    SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::Agent { socket },
    }
}

async fn dial(
    sshd: &support::sshd::TestSshd,
    cred: &SshCredential,
) -> Result<SshConnection, SshError> {
    SshConnection::connect(
        cred,
        HostKeyPolicy::AcceptNew {
            known_hosts: sshd.known_hosts_path(),
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_auth_succeeds_with_the_authorized_key_loaded() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd in this environment (honest skip, not a pass-fake)");
        return;
    };
    let Some(agent) = support::start_agent(&[sshd.client_key_path().as_path()]) else {
        eprintln!("SKIP: no usable ssh-agent/ssh-add in this environment");
        return;
    };
    let cred = agent_cred(&sshd, Some(agent.socket()));
    let conn = dial(&sshd, &cred).await.expect("agent auth");
    assert!(
        conn.fingerprint.as_deref().unwrap_or("").starts_with("SHA256:"),
        "should observe a SHA256 host fingerprint, got {:?}",
        conn.fingerprint
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_holding_only_unauthorized_keys_reports_how_many_it_tried() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    // The spare key is not in this host's authorized_keys.
    let Some(agent) = support::start_agent(&[sshd.spare_key_path().as_path()]) else {
        eprintln!("SKIP: no usable ssh-agent/ssh-add");
        return;
    };
    let cred = agent_cred(&sshd, Some(agent.socket()));
    let err = dial(&sshd, &cred)
        .await
        .expect_err("an unauthorized identity must not open the door");
    let msg = err.to_string();
    assert!(
        matches!(err, SshError::AuthFailed(_)),
        "expected AuthFailed, got {err:?}"
    );
    assert!(
        msg.contains('1') && msg.contains("identit"),
        "the failure must say how many identities were offered, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_agent_says_it_holds_no_identities() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let Some(agent) = support::start_agent(&[]) else {
        eprintln!("SKIP: no usable ssh-agent");
        return;
    };
    let cred = agent_cred(&sshd, Some(agent.socket()));
    let err = dial(&sshd, &cred).await.expect_err("empty agent");
    assert!(
        err.to_string().contains("no identities"),
        "an empty agent must be reported as such, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_agent_socket_is_reported_as_an_agent_problem() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let missing = std::env::temp_dir().join("nomi-ssh-agent-that-does-not-exist.sock");
    let cred = agent_cred(&sshd, Some(missing));
    let err = dial(&sshd, &cred).await.expect_err("no agent there");
    let msg = err.to_string();
    assert!(
        msg.contains("ssh-agent"),
        "must blame the agent, not the credential, got: {msg}"
    );
}
