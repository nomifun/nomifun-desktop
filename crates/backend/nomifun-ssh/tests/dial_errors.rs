//! Dial-error classification. No sshd needed: every case here is decided before
//! or at the TCP layer, and the point is that the classification is honest about
//! what a retry could possibly fix.
#[path = "support/mod.rs"]
mod support;

use nomifun_ssh::dto::CreateSshHostRequest;
use nomifun_ssh::{SshDialError, SshLinkKey, SshLinkState};

fn host_req(auth_type: &str, password: Option<&str>) -> CreateSshHostRequest {
    CreateSshHostRequest {
        name: "unreachable".into(),
        host: "127.0.0.1".into(),
        // Port 1 is reserved and never listening, so a dial that gets this far
        // fails at connect instead of hanging.
        port: 1,
        username: "nobody".into(),
        auth_type: auth_type.into(),
        password: password.map(str::to_string),
        private_key: None,
        passphrase: None,
        certificate: None,
        sudo_password: None,
    }
}

#[tokio::test]
async fn unknown_auth_type_is_a_non_retryable_dial_error() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    // The service validates auth_type on write, so an unknown one can only reach
    // the dialler from a row written by an older/newer build. Simulate that.
    let id = harness.add_host(host_req("password", Some("pw"))).await;
    sqlx::query("UPDATE ssh_hosts SET auth_type = 'quantum' WHERE ssh_host_id = ?")
        .bind(id.as_str())
        .execute(harness.service_pool())
        .await
        .expect("rewrite auth_type");

    let err = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("an unknown auth type cannot dial");
    assert!(
        matches!(err, SshDialError::Credential(_)),
        "expected a credential error, got {err:?}"
    );
    assert!(
        !err.is_retryable(),
        "retrying an unusable credential cannot help: {err:?}"
    );
}

#[tokio::test]
async fn missing_credential_maps_to_credential_error() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = harness.add_host(host_req("password", None)).await;

    let err = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("password auth without a password cannot dial");
    assert!(
        matches!(err, SshDialError::Credential(_)),
        "expected a credential error, got {err:?}"
    );
    assert!(!err.is_retryable(), "{err:?}");
}

#[tokio::test]
async fn a_refused_dial_publishes_dropped_with_the_retryable_flag() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = harness.add_host(host_req("password", Some("pw"))).await;
    let key = SshLinkKey {
        conversation_id: "conv-1".into(),
        ssh_host_id: id.clone(),
    };

    let err = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("port 1 is not listening");
    assert!(
        matches!(err, SshDialError::Unreachable(_)),
        "expected unreachable, got {err:?}"
    );
    assert!(err.is_retryable(), "a refused connection may come back");

    let state = harness
        .pool
        .subscribe(&key)
        .expect("a failed dial still leaves a link to watch")
        .borrow()
        .clone();
    match state {
        SshLinkState::Dropped { retryable, detail } => {
            assert!(retryable, "a refused connection is retryable: {detail}");
        }
        other => panic!("expected dropped, got {other:?}"),
    }
}

#[tokio::test]
async fn shutting_down_refuses_a_dial_before_touching_the_network() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = harness.add_host(host_req("password", Some("pw"))).await;

    let report = harness.pool.shutdown_all().await;
    assert_eq!(report.total(), 0, "nothing was open: {report:?}");

    let err = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("a shutting-down pool must refuse");
    assert!(matches!(err, SshDialError::ShuttingDown), "{err:?}");
    assert!(!err.is_retryable(), "{err:?}");
}
