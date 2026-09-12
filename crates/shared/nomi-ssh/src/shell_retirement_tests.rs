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
