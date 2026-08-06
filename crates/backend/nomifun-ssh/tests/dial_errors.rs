//! Dial-error classification. No sshd needed: every case here is decided before
//! or at the TCP layer, and the point is that the classification is honest about
//! what a retry could possibly fix.
#[path = "support/mod.rs"]
mod support;

use nomifun_ssh::dto::CreateSshHostRequest;
use nomifun_ssh::{SshDialError, SshLinkKey, SshLinkState};

/// Distinctive enough that a substring search for it is meaningful — the point of
/// a negative security assertion is that it would actually catch the leak.
const SECRET: &str = "hunter2_supersecret";

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
    let id = harness.add_host(host_req("password", Some(SECRET))).await;
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

    // The third face credentials can leak through. `Debug` is pinned by the
    // transport's own tests and the host-book DTO by `host_service.rs`; this is
    // the wire projection, whose `detail` is free-form operator text built from a
    // dial error and rendered verbatim in the status pill's popover.
    //
    // The host really does hold the secret — checked first, because an assertion
    // that a passwordless host leaks no password proves nothing at all.
    let cred = harness
        .service()
        .decrypt_credential(&harness.user_id, &id)
        .await
        .expect("the host stores a credential");
    assert_eq!(
        cred.password.as_ref().map(|p| p.as_str()),
        Some(SECRET),
        "the fixture must hold the secret whose absence is being asserted"
    );

    let snapshot = serde_json::to_string(&harness.pool.snapshot(&harness.user_id))
        .expect("serialize the status snapshot");
    assert!(
        !snapshot.contains(SECRET),
        "the status snapshot leaked the host password: {snapshot}"
    );
    let pushed = harness.events.status_payloads();
    assert!(
        !pushed.is_empty(),
        "the failed dial must have announced itself"
    );
    for payload in &pushed {
        assert!(
            !payload.to_string().contains(SECRET),
            "an ssh.status payload leaked the host password: {payload}"
        );
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

/// A port that completes the TCP handshake and then says nothing — the shape of a
/// mistyped port landing on a non-SSH service, and the case no layer below this
/// crate bounds: the transport sets no handshake timeout, so without a budget here
/// the dial never returns.
///
/// Not a firewall DROP, because that would need privileges; an accepted socket
/// that never sends an SSH banner reaches the same unbounded wait through the
/// same code path. The listener is owned by the test (a real `TcpListener` bound
/// to an ephemeral loopback port) and dropped with it — no child process, so
/// nothing to signal or clean up.
#[tokio::test]
async fn a_peer_that_never_speaks_ssh_times_out_instead_of_hanging() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a silent listener");
    let port = listener.local_addr().expect("addr").port();
    // Accept and hold, forever. Never writes the version string russh waits for.
    let _silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let mut req = host_req("password", Some("pw"));
    req.port = port as i64;
    let id = harness.add_host(req).await;

    let started = std::time::Instant::now();
    let err = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("a peer that never speaks ssh cannot produce a link");
    let waited = started.elapsed();

    assert!(
        matches!(err, SshDialError::Unreachable(_)),
        "a silent peer is unreachable, not a credential problem: {err:?}"
    );
    assert!(
        err.to_string().contains("timed out"),
        "the operator must be told the dial ran out of time: {err}"
    );
    assert!(
        err.is_retryable(),
        "a host that may simply be slow to answer stays retryable: {err:?}"
    );
    // The budget is 15s; anything near the kernel's SYN-retry horizon (~130s) or
    // beyond means nothing bounded the dial.
    assert!(
        waited < std::time::Duration::from_secs(30),
        "the dial must be bounded by its own budget, waited {waited:?}"
    );
}
