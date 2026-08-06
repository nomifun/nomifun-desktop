//! OpenSSH certificate authentication against a real throwaway sshd whose
//! `TrustedUserCAKeys` is the fixture's own CA and whose `authorized_keys` is
//! empty — so a login here can only have come from the certificate.
//!
//! The negative cases pin the *diagnosis*, not just the failure: "authentication
//! failed" tells an operator nothing, whereas "valid for principals [x], you
//! connected as y" tells them exactly which knob to turn.
mod support;

use nomi_ssh::connection::{HostKeyPolicy, SshConnection, SshError};
use nomi_ssh::credential::{Auth, SshCredential};

fn cert_cred(sshd: &support::sshd::TestSshd, cert: String) -> SshCredential {
    SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::Certificate {
            key_pem: sshd.client_key_pem(),
            cert,
            passphrase: None,
        },
    }
}

async fn dial(sshd: &support::sshd::TestSshd, cred: &SshCredential) -> Result<SshConnection, SshError> {
    SshConnection::connect(
        cred,
        HostKeyPolicy::AcceptNew {
            known_hosts: sshd.known_hosts_path(),
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn certificate_auth_succeeds_against_a_ca_the_server_trusts() {
    let Some(sshd) = support::start_cert_sshd() else {
        eprintln!("SKIP: no usable sshd/ssh-keygen in this environment (honest skip, not a pass-fake)");
        return;
    };
    let cred = cert_cred(&sshd, sshd.client_cert());
    let conn = dial(&sshd, &cred).await.expect("certificate auth");
    assert!(
        conn.fingerprint.as_deref().unwrap_or("").starts_with("SHA256:"),
        "should observe a SHA256 host fingerprint, got {:?}",
        conn.fingerprint
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_issued_to_another_principal_names_the_principal_mismatch() {
    let Some(sshd) = support::start_cert_sshd() else {
        eprintln!("SKIP: no usable sshd/ssh-keygen");
        return;
    };
    let cred = cert_cred(&sshd, sshd.cert_for_another_principal());
    let err = dial(&sshd, &cred)
        .await
        .expect_err("a cert for another principal must not open the door");
    let msg = err.to_string();
    assert!(
        matches!(err, SshError::AuthFailed(_)),
        "expected AuthFailed, got {err:?}"
    );
    assert!(
        msg.contains("principal") && msg.contains("nomi-not-this-user"),
        "the failure must name the principal mismatch, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_certificate_says_it_expired() {
    let Some(sshd) = support::start_cert_sshd() else {
        eprintln!("SKIP: no usable sshd/ssh-keygen");
        return;
    };
    let cred = cert_cred(&sshd, sshd.expired_cert());
    let err = dial(&sshd, &cred)
        .await
        .expect_err("an expired cert must not open the door");
    let msg = err.to_string();
    assert!(
        matches!(err, SshError::AuthFailed(_)),
        "expected AuthFailed, got {err:?}"
    );
    assert!(
        msg.contains("expired"),
        "the failure must say the certificate expired, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unusable_certificate_material_is_refused_with_a_precise_message() {
    let Some(sshd) = support::start_cert_sshd() else {
        eprintln!("SKIP: no usable sshd/ssh-keygen");
        return;
    };

    // Not a certificate at all — the single most likely paste error.
    let garbage = cert_cred(&sshd, "ssh-ed25519 AAAAnot-a-certificate wat\n".into());
    let err = dial(&sshd, &garbage).await.expect_err("garbage cert");
    assert!(
        err.to_string().contains("certificate"),
        "a bad cert body must be reported as a certificate problem, got: {err}"
    );

    // A real certificate, but for a different key than the one supplied.
    let mut mismatched = cert_cred(&sshd, sshd.client_cert());
    mismatched.auth = Auth::Certificate {
        key_pem: sshd.spare_key_pem(),
        cert: sshd.client_cert(),
        passphrase: None,
    };
    let err = dial(&sshd, &mismatched)
        .await
        .expect_err("cert that does not belong to the key");
    assert!(
        err.to_string().contains("does not match"),
        "a key/cert mismatch must say so, got: {err}"
    );
}
