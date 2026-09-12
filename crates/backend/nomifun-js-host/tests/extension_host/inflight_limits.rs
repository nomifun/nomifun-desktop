use super::*;

async fn limited_host(limit: usize, timeout: Duration) -> ExtensionHostSupervisor {
    let mut config = host_config(
        timeout,
        fixture("../../assets/extension-host.mjs")
            .canonicalize()
            .unwrap(),
    )
    .await;
    config.limits.max_pending_requests = limit;
    ExtensionHostSupervisor::new(config).unwrap()
}

#[tokio::test]
async fn work_limit_preserves_cancellation_and_releases_completed_slots() {
    let host = limited_host(1, Duration::from_secs(3)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let pending = host
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("wait_for_cancel"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    let overflow = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({})),
        )
        .await;
    // Cleanup also runs on the old implementation before asserting overflow.
    host.cancel(pending.request_id.clone()).await.unwrap();
    let cancelled = pending.wait().await;
    let recovered = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({"recovered": true})),
        )
        .await;
    host.stop_generation(generation).await.unwrap();
    assert_eq!(overflow.unwrap_err(), JavaScriptHostError::QueueFull);
    assert!(matches!(
        cancelled,
        Err(JavaScriptHostError::RequestFailed { .. })
    ));
    assert_eq!(recovered.unwrap().0["input"]["recovered"], true);
}

#[tokio::test]
async fn coalesced_callers_count_toward_the_work_limit() {
    let host = limited_host(2, Duration::from_secs(1)).await;
    let temp = TempDir::new().unwrap();
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"activation": "hang"}));
    let demand = MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    };
    let (a, b, c, d, e) = tokio::join!(
        host.load_mount(demand.clone()),
        host.load_mount(demand.clone()),
        host.load_mount(demand.clone()),
        host.load_mount(demand.clone()),
        host.load_mount(demand),
    );
    wait_until_stopped(&host).await;
    let errors = [a, b, c, d, e].map(Result::unwrap_err);
    assert_eq!(
        errors
            .iter()
            .filter(|e| **e == JavaScriptHostError::QueueFull)
            .count(),
        3,
        "coalesced callers bypassed capacity: {errors:?}"
    );
    assert_eq!(
        errors
            .iter()
            .filter(|e| matches!(e, JavaScriptHostError::HostFailure { .. }))
            .count(),
        2
    );
}
