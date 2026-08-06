//! What `changedAt` means on the wire.
//!
//! No sshd needed: a dial that fails still drives a link through real
//! transitions, and the question here is not how it failed but whether the
//! timestamp the client uses for ordering and countdowns describes the *link* or
//! the *request*.
#[path = "support/mod.rs"]
mod support;

use std::time::Duration;

use nomifun_ssh::dto::CreateSshHostRequest;

fn unreachable_host() -> CreateSshHostRequest {
    CreateSshHostRequest {
        name: "unreachable".into(),
        host: "127.0.0.1".into(),
        // Reserved and never listening: the dial fails at connect, so the link
        // reaches `Dropped` promptly and then stops changing.
        port: 1,
        username: "nobody".into(),
        auth_type: "password".into(),
        password: Some("pw".into()),
        private_key: None,
        passphrase: None,
        certificate: None,
        sudo_password: None,
    }
}

#[tokio::test]
async fn a_snapshot_reports_when_the_link_changed_not_when_it_was_asked() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = harness.add_host(unreachable_host()).await;

    harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("port 1 is not listening");

    let first = harness.pool.snapshot(&harness.user_id);
    assert_eq!(first.len(), 1, "one link, one row: {first:?}");
    // Long enough that a per-request `now_ms()` cannot coincide with the first.
    tokio::time::sleep(Duration::from_millis(25)).await;
    let second = harness.pool.snapshot(&harness.user_id);

    assert_eq!(
        first[0].changed_at, second[0].changed_at,
        "the link did not change between the two snapshots, so `changedAt` must \
         not move — the client uses it to discard out-of-order deliveries and to \
         anchor the reconnect countdown, and a re-read that always looks newest \
         defeats both"
    );

    // Push and snapshot must agree, because they are the same fact: the client
    // mixes events with re-fetches and compares their timestamps to each other.
    let pushed = harness.events.status_payloads();
    let last = pushed.last().expect("the drop was announced");
    assert_eq!(
        last["changedAt"].as_i64(),
        Some(first[0].changed_at),
        "the emitted event and the snapshot describe one transition: {last}"
    );
}
