use super::*;
use std::sync::Arc;

async fn wait_for_state(host: &ExtensionHostSupervisor, field: &str, expected: u64) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let value = host.invoke(
                contribution(target("mount-a", 'a')),
                ActionId::from("cancellation_state"),
                StrictJsonValue(json!({})),
            ).await.unwrap();
            if value.0[field] == expected { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("fixture did not reach the expected cancellation state");
}

#[tokio::test]
async fn dropped_tool_handles_cancel_only_abandoned_work() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let retained = host.start_invocation(
        contribution(target("mount-a", 'a')),
        ActionId::from("wait_for_cancel"),
        StrictJsonValue(json!({})),
    ).await.unwrap();
    let mut abandoned = Vec::new();
    for _ in 0..4 {
        abandoned.push(host.start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("wait_for_cancel"),
            StrictJsonValue(json!({})),
        ).await.unwrap());
    }
    wait_for_state(&host, "tool_started", 5).await;
    drop(abandoned);
    wait_for_state(&host, "tool_cancelled", 4).await;

    // Live waiters are not canceled, even on the same Mount. Multiple dropped
    // requests must pass through the one reserved cancellation lane.
    host.cancel(retained.request_id.clone()).await.unwrap();
    assert!(matches!(retained.wait().await, Err(JavaScriptHostError::RequestFailed { .. })));
    wait_for_state(&host, "tool_cancelled", 5).await;
    assert_eq!(host.process_count(), 1);
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn dropped_context_wait_cancels_js_without_retiring_mount() {
    let host = Arc::new(supervisor(Duration::from_secs(5)).await);
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let pending = tokio::spawn({
        let host = Arc::clone(&host);
        async move {
            host.contribute_context(
                contribution(target("mount-a", 'a')),
                CanonicalSchemaRef::from("schema://fixture/wait-for-cancel"),
            ).await
        }
    });
    wait_for_state(&host, "context_started", 1).await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    wait_for_state(&host, "context_cancelled", 1).await;

    let value = host.contribute_context(
        contribution(target("mount-a", 'a')),
        CanonicalSchemaRef::from("schema://fixture/context"),
    ).await.unwrap();
    assert_eq!(value.0["mount_id"], "mount-a");
    assert!(matches!(host.state(), JavaScriptHostState::Running { generation: current, .. } if current == generation));
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn abandoned_noncooperative_work_still_hits_watchdog() {
    let host = supervisor(Duration::from_millis(500)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let pending = host.start_invocation(
        contribution(target("mount-a", 'a')),
        ActionId::from("hang"),
        StrictJsonValue(json!({})),
    ).await.unwrap();
    // Ensure the work was admitted before abandoning it. Cancellation ACK must
    // not retire its original request or extend the watchdog deadline.
    host.invoke(
        contribution(target("mount-a", 'a')),
        ActionId::from("echo"),
        StrictJsonValue(json!({})),
    ).await.unwrap();
    drop(pending);
    wait_until_stopped(&host).await;
    assert!(matches!(host.state(), JavaScriptHostState::Failed { generation: current, reason }
        if current == generation && reason.contains("watchdog timed out")));
    assert_eq!(host.process_count(), 0);
}
