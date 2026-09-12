use super::protocol_tests::{connect_peer, connect_peer_with_window, finish_peer};
use super::*;
use tokio::time::{Instant, timeout};

#[tokio::test]
async fn command_budget_includes_waiting_for_the_operation_slot() {
    let (connection, task) = connect_peer(0).await;
    let shell = connection.open_shell(".").await.unwrap();
    let lock = shell.operation.lock().await;
    let result = timeout(
        Duration::from_millis(200),
        shell.run("true", Duration::from_millis(30)),
    )
    .await;
    drop(lock);
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert!(
        matches!(result, Ok(Err(SshError::TimedOut(_)))),
        "lock admission must honor the caller budget: {result:?}"
    );
    assert!(reusable, "a queued call did not touch the channel");
}

#[tokio::test]
async fn command_budget_includes_a_stalled_payload_write() {
    let (connection, task) = connect_peer_with_window(0, 1024).await;
    let shell = connection.open_shell(".").await.unwrap();
    let result = timeout(
        Duration::from_millis(200),
        shell.run(&"x".repeat(64 * 1024), Duration::from_millis(30)),
    )
    .await;
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert!(
        matches!(result, Ok(Err(SshError::TimedOut(_)))),
        "partial submission is an unknown outcome, not an unbounded wait: {result:?}"
    );
    assert!(!reusable);
}

#[tokio::test]
async fn close_budget_includes_a_stalled_exit_write() {
    let (connection, task) = connect_peer_with_window(0, 1024).await;
    let shell = connection.open_shell(".").await.unwrap();
    {
        let channel = shell.channel.lock().unwrap().channel.take().unwrap();
        let stalled = timeout(
            Duration::from_millis(30),
            channel.data_bytes(vec![b'x'; 64 * 1024]),
        )
        .await
        .is_err();
        shell.channel.lock().unwrap().channel = Some(channel);
        assert!(stalled);
    }
    let result = timeout(
        Duration::from_millis(200),
        shell.close(Duration::from_millis(30)),
    )
    .await;
    finish_peer(&connection, task).await;
    let proof = result.expect("close must stop despite a zero sending window");
    assert!(!proof.is_reaped());
    assert!(!proof.errors.is_empty());
}

#[tokio::test]
async fn close_does_not_restart_its_budget_after_lock_admission() {
    let (connection, task) = connect_peer(0).await;
    let shell = connection.open_shell(".").await.unwrap();
    let lock = shell.operation.lock().await;
    let started = Instant::now();
    let release = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(lock);
    };
    let (_, proof) = tokio::join!(release, shell.close(Duration::from_millis(200)));
    let elapsed = started.elapsed();
    finish_peer(&connection, task).await;
    assert!(
        elapsed < Duration::from_millis(300),
        "lock + close restarted the budget: {elapsed:?}"
    );
    assert!(!proof.is_reaped());
}

#[tokio::test]
async fn initialization_budget_includes_its_first_write() {
    let (connection, task) = connect_peer_with_window(0, 0).await;
    let result = timeout(
        INIT_READY_TIMEOUT + Duration::from_millis(500),
        connection.open_shell("."),
    )
    .await;
    finish_peer(&connection, task).await;
    assert!(
        matches!(result, Ok(Err(SshError::TimedOut(_)))),
        "initialization must stop even before a sentinel can be sent"
    );
}
#[tokio::test]
async fn prompt_answer_and_recovery_writes_share_bounded_deadlines() {
    let (connection, task) = connect_peer_with_window(0, 1024).await;
    let shell = connection
        .open_shell_with_rules(
            ".",
            vec![AnswerRule::sudo(zeroize::Zeroizing::new(
                "x".repeat(64 * 1024),
            ))],
        )
        .await
        .unwrap();
    let result = timeout(
        DRAIN_TIMEOUT + Duration::from_millis(500),
        shell.run("fixture_prompt", Duration::from_millis(30)),
    )
    .await;
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    let outcome = result
        .expect("neither answer nor recovery can block outside the budget")
        .unwrap();
    assert!(outcome.timed_out);
    assert_eq!(outcome.output, "Password: ");
    assert!(outcome.cwd.is_empty());
    assert!(!reusable);
}

#[tokio::test]
async fn successful_command_and_timeout_resynchronization_remain_reusable() {
    let (connection, task) = connect_peer(0).await;
    let shell = connection.open_shell(".").await.unwrap();
    let success = shell
        .run("fixture_success", Duration::from_secs(1))
        .await
        .unwrap();
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert_eq!(success.output, "first");
    assert!(!success.timed_out);
    assert!(reusable);

    let (connection, task) = connect_peer(0).await;
    let shell = connection.open_shell(".").await.unwrap();
    let recovered = shell
        .run("fixture_silent", Duration::from_millis(30))
        .await
        .unwrap();
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert!(recovered.timed_out);
    assert_eq!(recovered.cwd, "/requested-directory");
    assert!(reusable);
}
