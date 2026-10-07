use super::protocol_tests::{connect_observed_peer, finish_peer};
use super::*;
use tokio::time::timeout;

#[tokio::test]
async fn cancelling_a_partial_write_can_still_close_the_channel() {
    let (connection, task, closed) = connect_observed_peer(0, 1024).await;
    let shell = connection.open_shell(".").await.unwrap();
    let result = timeout(
        Duration::from_millis(30),
        shell.run(&"x".repeat(64 * 1024), Duration::from_secs(30)),
    )
    .await;
    let observed = timeout(Duration::from_millis(200), closed.notified()).await;
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert!(result.is_err(), "fixture cancelled the blocked write");
    assert!(!reusable);
    assert!(
        observed.is_ok(),
        "close must bypass exhausted data window; connection remains alive during observation"
    );
}

#[tokio::test]
async fn failed_initialization_does_not_abandon_an_open_channel() {
    let (connection, task, closed) = connect_observed_peer(2, 2 * 1024 * 1024).await;
    let result = connection.open_shell(".").await;
    let observed = timeout(Duration::from_millis(200), closed.notified()).await;
    finish_peer(&connection, task).await;
    assert!(result.is_err());
    assert!(
        observed.is_ok(),
        "initialization failure must request channel closure"
    );
}

#[tokio::test]
async fn dropping_an_idle_shell_requests_channel_close_without_a_task() {
    let (connection, task, closed) = connect_observed_peer(0, 2 * 1024 * 1024).await;
    let shell = connection.open_shell(".").await.unwrap();
    drop(shell);
    let observed = timeout(Duration::from_millis(200), closed.notified()).await;
    finish_peer(&connection, task).await;
    assert!(
        observed.is_ok(),
        "dropping a shell must not silently drop only its local receiver"
    );
}
#[tokio::test]
async fn explicit_close_can_read_proof_after_cancellation() {
    let (connection, task, _) = super::protocol_tests::connect_scripted_peer(0, 1024, true).await;
    let shell = connection.open_shell(".").await.unwrap();
    let cancelled = timeout(
        Duration::from_millis(30),
        shell.run(&"x".repeat(64 * 1024), Duration::from_secs(30)),
    )
    .await;
    let proof = shell.close(Duration::from_millis(500)).await;
    finish_peer(&connection, task).await;
    assert!(cancelled.is_err());
    assert!(
        proof.is_reaped(),
        "a cancelled lease must retain the actual peer exit evidence: {proof:?}"
    );
    assert_eq!(proof.exit_status, Some(143));
}

#[tokio::test]
async fn explicit_normal_close_records_exit_status_and_peer_close() {
    let (connection, task, _) =
        super::protocol_tests::connect_scripted_peer(0, 2 * 1024 * 1024, true).await;
    let shell = connection.open_shell(".").await.unwrap();
    let proof = shell.close(Duration::from_millis(500)).await;
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert!(proof.is_reaped(), "{proof:?}");
    assert_eq!(proof.exit_status, Some(0));
    assert!(!reusable);
}

#[tokio::test]
async fn terminal_run_receipt_is_retained_for_idempotent_close() {
    let (connection, task) = super::protocol_tests::connect_peer(0).await;
    let shell = connection.open_shell(".").await.unwrap();
    let outcome = shell.run("fixture_terminal_exit", Duration::from_secs(1)).await.unwrap();
    assert_eq!(outcome.exit_code, 7);
    assert_eq!(outcome.output, "terminal_marker");
    assert!(outcome.cwd.is_empty());
    assert!(!shell.is_reusable().await);
    let proof = shell.close(Duration::from_millis(200)).await;
    let repeated = shell.close(Duration::from_millis(200)).await;
    finish_peer(&connection, task).await;
    assert!(proof.is_reaped(), "the terminal messages consumed by run are exact close evidence: {proof:?}");
    assert_eq!(proof.exit_status, Some(7));
    assert_eq!(repeated, proof);
}

#[tokio::test]
async fn close_admission_timeout_cannot_downgrade_retained_terminal_proof() {
    let (connection, task) = super::protocol_tests::connect_peer(0).await;
    let shell = connection.open_shell(".").await.unwrap();
    shell.run("fixture_terminal_exit", Duration::from_secs(1)).await.unwrap();
    let locked = shell.operation.lock().await;
    let timeout_proof = shell.close(Duration::from_millis(20)).await;
    drop(locked);
    let retained = shell.close(Duration::from_millis(200)).await;
    finish_peer(&connection, task).await;
    assert!(!timeout_proof.errors.is_empty());
    assert!(retained.is_reaped(), "a non-admitted closer must not erase proof: {retained:?}");
    assert_eq!(retained.exit_status, Some(7));
    assert!(retained.errors.is_empty(), "admission errors do not belong to the proven close receipt");
}
