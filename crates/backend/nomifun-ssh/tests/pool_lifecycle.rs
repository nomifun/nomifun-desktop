//! Pool lifecycle against a real sshd: reuse, the published transition sequence,
//! the reconnect ladder with cwd replay, shell recycling, and close-with-forensics.
//!
//! Every test self-skips (printing `SKIP:`) when no usable sshd is on the box, or
//! when the box is too loaded to start a remote shell inside `nomi-ssh`'s init
//! budget — a fake pass here would hide exactly the failures this file exists to
//! catch, and a fake *failure* would hide them just as well by crying wolf.
//!
//! Sequences are asserted against the emitted `ssh.status` log rather than the
//! `watch`: the log is append-only, so it cannot coalesce away a phase the pool
//! really did go through. The `watch` is used for *waiting*, which is what it is
//! good at.
#[path = "support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use nomifun_common::OnConversationDelete;
use nomifun_ai_agent::{SshBackendProvider, SshLeaseRelease};
use nomifun_ssh::dto::CreateSshHostRequest;
use nomifun_ssh::{SshDialError, SshLinkKey, SshLinkPhase, SshTeardown};

/// Generous by test standards, tight by SSH standards: a local dial is tens of
/// milliseconds, so anything near this budget is a hang, not slowness. The margin
/// is for a busy box, not for a slow implementation.
const SETTLE: Duration = Duration::from_secs(45);

macro_rules! sshd_or_skip {
    ($name:expr) => {
        match support::sshd::start_pubkey_sshd() {
            Some(sshd) => sshd,
            None => {
                println!("SKIP: {}: no usable sshd/ssh-keygen on this machine", $name);
                return;
            }
        }
    };
}

fn key(conversation_id: &str, ssh_host_id: &nomifun_common::SshHostId) -> SshLinkKey {
    SshLinkKey {
        conversation_id: conversation_id.to_string(),
        ssh_host_id: ssh_host_id.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acquire_twice_returns_the_same_link() {
    const NAME: &str = "acquire_twice_returns_the_same_link";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    let Some(first) = harness.open_or_skip(NAME, "conv-1", &id, "/").await else {
        return;
    };
    let second = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect("second acquire");

    assert!(
        Arc::ptr_eq(&first, &second),
        "the same conversation+host must reuse one link, not open a second socket"
    );
    assert_eq!(harness.pool.active_link_count(), 1);
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_publishes_connecting_then_connected() {
    const NAME: &str = "connect_publishes_connecting_then_connected";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;
    let link_key = key("conv-1", &id);

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }

    assert_eq!(
        harness.events.status_phases(),
        vec!["connecting".to_string(), "connected".to_string()],
        "a first dial publishes exactly connecting then connected"
    );
    let state = harness
        .pool
        .subscribe(&link_key)
        .expect("link exists")
        .borrow()
        .clone();
    assert_eq!(
        state.phase(),
        SshLinkPhase::Connected,
        "and the watch agrees with what was emitted: {state:?}"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_emits_ssh_status_to_the_owner_only() {
    const NAME: &str = "connect_emits_ssh_status_to_the_owner_only";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }

    let addressed = harness.events.addressed();
    assert!(!addressed.is_empty(), "a dial must announce itself");
    for (recipient, event) in &addressed {
        assert_eq!(
            recipient, &harness.user_id,
            "link status leaked to {recipient}"
        );
        assert_eq!(event, "ssh.status");
    }
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_transport_publishes_dropped_then_reconnecting_then_connected() {
    const NAME: &str = "a_dead_transport_publishes_dropped_then_reconnecting_then_connected";
    let mut sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;
    let link_key = key("conv-1", &id);

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    let mut rx = harness.pool.subscribe(&link_key).expect("link exists");
    assert_eq!(rx.borrow_and_update().phase(), SshLinkPhase::Connected);

    sshd.stop();
    support::collect_phases_until(&mut rx, SshLinkPhase::Reconnecting, SETTLE).await;
    sshd.restart().expect("restart the fixture sshd on the same port");
    support::collect_phases_until(&mut rx, SshLinkPhase::Connected, SETTLE).await;

    assert!(
        harness.events.saw_phases_in_order(&["connected", "dropped"]),
        "the supervisor must report the loss before it starts retrying: {:?}",
        harness.events.status_phases()
    );
    harness
        .events
        .await_phases_in_order(
            &["connected", "dropped", "reconnecting", "connected"],
            SETTLE,
        )
        .await;

    // And the link is genuinely usable again, not merely labelled connected.
    let link = harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect("reuse after recovery");
    let out = harness
        .pool
        .backend_for(&link)
        .run_command("echo alive", 15_000)
        .await
        .expect("command on the recovered link");
    assert!(out.stdout.contains("alive"), "{out:?}");
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_replays_the_last_proven_cwd() {
    const NAME: &str = "reconnect_replays_the_last_proven_cwd";
    let mut sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;
    let link_key = key("conv-1", &id);

    let Some(link) = harness.open_or_skip(NAME, "conv-1", &id, "/").await else {
        return;
    };
    let backend = harness.pool.backend_for(&link);
    backend
        .run_command("cd /tmp", 15_000)
        .await
        .expect("cd should succeed");
    assert_eq!(
        link.last_cwd(),
        "/tmp",
        "the sentinel proved the cwd; the pool must remember it"
    );

    let mut rx = harness.pool.subscribe(&link_key).expect("link exists");
    rx.borrow_and_update();
    sshd.stop();
    support::collect_phases_until(&mut rx, SshLinkPhase::Reconnecting, SETTLE).await;
    sshd.restart().expect("restart");
    support::collect_phases_until(&mut rx, SshLinkPhase::Connected, SETTLE).await;

    let out = backend
        .run_command("pwd", 15_000)
        .await
        .expect("command after reconnect");
    assert!(
        out.stdout.contains("/tmp"),
        "a reconnect must not silently move the model back to $HOME: {out:?}"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unrecoverable_shell_is_recycled_without_redialling() {
    const NAME: &str = "an_unrecoverable_shell_is_recycled_without_redialling";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    let Some(link) = harness.open_or_skip(NAME, "conv-1", &id, "/").await else {
        return;
    };
    let backend = harness.pool.backend_for(&link);
    backend
        .run_command("cd /tmp", 15_000)
        .await
        .expect("cd should succeed");

    // A command that ignores the interrupt (the disposition is inherited across
    // exec, so `sleep` ignores it too) and outlives the drain window leaves
    // `RemoteShell::run` unable to resynchronize: the shell is unusable while the
    // transport is perfectly fine.
    let out = backend
        .run_command("trap '' INT; sleep 30", 700)
        .await
        .expect("a timeout is an outcome, not an error");
    assert!(out.timed_out, "{out:?}");

    // A redial would have to publish `connecting`; there is none between the
    // degradation and the recovery, which is what proves the transport was reused.
    let phases = harness.events.status_phases();
    let tail = &phases[phases.len().saturating_sub(2)..];
    assert_eq!(
        tail,
        ["degraded".to_string(), "connected".to_string()],
        "a wedged shell must be reopened on the same transport: {phases:?}"
    );

    let after = backend
        .run_command("pwd", 15_000)
        .await
        .expect("the recycled shell works");
    assert!(
        after.stdout.contains("/tmp"),
        "the proven cwd must survive a recycle: {after:?}"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_link_publishes_closed_with_a_reaped_teardown() {
    const NAME: &str = "close_link_publishes_closed_with_a_reaped_teardown";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;
    let link_key = key("conv-1", &id);

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    let teardown = harness.pool.close_link(&link_key).await;
    match &teardown {
        SshTeardown::Reaped { detail } => assert!(
            detail.contains("exit"),
            "a reaped close must cite its evidence: {detail}"
        ),
        other => panic!("a healthy link closes reaped, got {other:?}"),
    }
    assert_eq!(harness.pool.active_link_count(), 0);
    assert_eq!(
        harness.events.status_phases().last().map(String::as_str),
        Some("closed"),
        "the close must reach the operator too"
    );
    assert!(
        harness.pool.subscribe(&link_key).is_none(),
        "a closed link must not linger in the pool"
    );
}

/// The seam the agent factory calls, against a real host: one pooled link, tools
/// that work through it, and a lease that reports rather than closes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_lease_keeps_the_link_and_reports_the_close_only_afterwards() {
    const NAME: &str = "a_released_lease_keeps_the_link_and_reports_the_close_only_afterwards";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;
    let link_key = key("conv-1", &id);

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    let provider: Arc<dyn SshBackendProvider> = Arc::new(harness.pool.clone());
    let binding = provider
        .connect(&harness.user_id, "conv-1", id.as_str(), "/")
        .await
        .expect("the seam must hand back the conversation's pooled link");
    assert_eq!(
        harness.pool.active_link_count(),
        1,
        "the seam must reuse the link the session already had, not dial a second one"
    );
    let echoed = binding
        .backend
        .run_command("echo lease_ok", 30_000)
        .await
        .expect("the seam's backend must reach the host");
    assert!(echoed.stdout.contains("lease_ok"), "{:?}", echoed.stdout);

    // A model switch destroys the runtime and releases its lease. Releasing is not
    // closing: the operator's shell — and its cwd — must still be there.
    match binding.lease.release().await {
        SshLeaseRelease::Retained { .. } => {}
        other => panic!("releasing a live lease must retain the link, got {other:?}"),
    }
    assert!(
        harness.pool.is_pooled(&link_key),
        "the released link must still belong to the conversation"
    );

    // Only when the pool itself closes the link does the same lease report
    // forensics — and then it reports the pool's proof, not a guess.
    match harness.pool.close_link(&link_key).await {
        SshTeardown::Reaped { .. } => {}
        other => panic!("a healthy link closes reaped, got {other:?}"),
    }
    match binding.lease.release().await {
        SshLeaseRelease::Reaped { detail } => assert!(
            detail.contains("exit"),
            "a reaped release must carry the pool's evidence: {detail}"
        ),
        other => panic!("a proven close must release as reaped, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_link_on_an_already_dropped_link_is_not_reaped() {
    const NAME: &str = "close_link_on_an_already_dropped_link_is_not_reaped";
    let mut sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;
    let link_key = key("conv-1", &id);

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    let mut rx = harness.pool.subscribe(&link_key).expect("link exists");
    rx.borrow_and_update();
    // The host stays down, so once the link leaves `Connected` it cannot come back
    // — which is what makes this deterministic rather than a race with the ladder.
    sshd.stop();
    support::collect_phases_until(&mut rx, SshLinkPhase::Reconnecting, SETTLE).await;

    let teardown = harness.pool.close_link(&link_key).await;
    assert!(
        matches!(teardown, SshTeardown::AlreadyDown { .. }),
        "there was nothing left to reap, and pretending otherwise is the one lie \
         teardown reporting must never tell: {teardown:?}"
    );
    assert_eq!(harness.pool.active_link_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_conversation_closes_every_host_link() {
    const NAME: &str = "close_conversation_closes_every_host_link";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    // Two host-book entries pointing at the same box: two links under one
    // conversation, which is what rebinding a session leaves behind.
    let first = harness.add_fixture_host(&sshd).await;
    let second = harness.add_fixture_host(&sshd).await;

    if harness
        .open_or_skip(NAME, "conv-1", &first, "/")
        .await
        .is_none()
    {
        return;
    }
    harness
        .pool
        .acquire(&harness.user_id, "conv-1", &second, "/")
        .await
        .expect("second link");
    harness
        .pool
        .acquire(&harness.user_id, "conv-2", &first, "/")
        .await
        .expect("other conversation");
    assert_eq!(harness.pool.active_link_count(), 3);

    let teardowns = harness.pool.close_conversation("conv-1").await;
    assert_eq!(teardowns.len(), 2, "{teardowns:?}");
    assert!(
        teardowns
            .iter()
            .all(|t| matches!(t, SshTeardown::Reaped { .. })),
        "{teardowns:?}"
    );
    assert_eq!(
        harness.pool.active_link_count(),
        1,
        "the other conversation's link must survive"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_all_refuses_new_acquires_and_counts_reaped() {
    const NAME: &str = "shutdown_all_refuses_new_acquires_and_counts_reaped";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    let report = harness.pool.shutdown_all().await;
    assert_eq!(report.reaped, 1, "{report:?}");
    assert_eq!(report.lost, 0, "{report:?}");
    assert_eq!(harness.pool.active_link_count(), 0);

    let err = harness
        .pool
        .acquire(&harness.user_id, "conv-2", &id, "/")
        .await
        .expect_err("a shut-down pool must not open new sockets");
    assert!(matches!(err, SshDialError::ShuttingDown), "{err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_conversation_deleted_closes_the_link() {
    const NAME: &str = "on_conversation_deleted_closes_the_link";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    // Through the trait object, because that is how the conversation service will
    // reach the pool.
    let hook: Arc<dyn OnConversationDelete> = Arc::new(harness.pool.clone());
    hook.on_conversation_deleted(&harness.user_id, "conv-1")
        .await;

    assert_eq!(harness.pool.active_link_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_for_host_drops_its_links_and_stops_redialling() {
    const NAME: &str = "close_for_host_drops_its_links_and_stops_redialling";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    if harness.open_or_skip(NAME, "conv-1", &id, "/").await.is_none() {
        return;
    }
    harness
        .pool
        .acquire(&harness.user_id, "conv-2", &id, "/")
        .await
        .expect("second conversation");
    assert_eq!(harness.pool.active_link_count(), 2);

    harness.pool.close_for_host(&id).await;
    assert_eq!(harness.pool.active_link_count(), 0);

    // A deliberate new acquire is the operator asking again, so it clears the
    // retirement; supervisors never get that privilege.
    harness
        .pool
        .acquire(&harness.user_id, "conv-3", &id, "/")
        .await
        .expect("a deliberate acquire may dial a re-authorized host");
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_reports_the_fingerprint_of_a_reachable_host() {
    const NAME: &str = "probe_reports_the_fingerprint_of_a_reachable_host";
    let sshd = sshd_or_skip!(NAME);
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let id = harness.add_fixture_host(&sshd).await;

    let outcome = harness.pool.probe(&harness.user_id, &id).await;
    if outcome.detail.contains("did not become ready") {
        println!("SKIP: {NAME}: {}", outcome.detail);
        return;
    }
    assert!(outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .host_fingerprint
            .as_deref()
            .is_some_and(|f| f.starts_with("SHA256:")),
        "{outcome:?}"
    );
    assert_eq!(
        harness.pool.active_link_count(),
        0,
        "a probe must not leave a pooled link behind"
    );
}

#[tokio::test]
async fn mark_unreachable_walks_back_the_status_without_losing_last_connected_at() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = harness
        .add_host(CreateSshHostRequest {
            name: "host".into(),
            host: "127.0.0.1".into(),
            port: 1,
            username: "nobody".into(),
            auth_type: "password".into(),
            password: Some("pw".into()),
            private_key: None,
            passphrase: None,
            certificate: None,
            sudo_password: None,
        })
        .await;
    let service = harness.service();
    service
        .mark_connected(&harness.user_id, &id, Some("SHA256:seen"))
        .await
        .expect("mark connected");
    let connected_at = service
        .get(&harness.user_id, &id)
        .await
        .expect("get")
        .last_connected_at;
    assert!(connected_at.is_some());

    service
        .mark_unreachable(&harness.user_id, &id, "connection refused")
        .await
        .expect("mark unreachable");

    let row = service.get(&harness.user_id, &id).await.expect("get");
    assert_eq!(row.status, "disconnected");
    assert_eq!(
        row.last_connected_at, connected_at,
        "the hint about when it last worked must survive the walk-back"
    );
    assert_eq!(
        row.host_fingerprint.as_deref(),
        Some("SHA256:seen"),
        "the known fingerprint must survive too"
    );
}
