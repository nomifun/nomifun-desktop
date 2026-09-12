use super::*;

async fn acquire(
    host: &ExtensionHostSupervisor,
    binding: &str,
    parameters: serde_json::Value,
) -> nomifun_js_host::JavaScriptResourceHandle {
    host.acquire_resource(
        contribution(target("mount-a", 'a')),
        ResourceBindingId::from(binding),
        ResourceKind::from("fixture.resource"),
        StrictJsonValue(parameters),
    )
    .await
    .unwrap()
}

async fn release_count(host: &ExtensionHostSupervisor) -> serde_json::Value {
    host.invoke(
        contribution(target("mount-a", 'a')),
        ActionId::from("resource_release_count"),
        StrictJsonValue(json!({})),
    )
    .await
    .unwrap()
    .0["count"]
        .clone()
}

#[tokio::test]
async fn rejected_acquisitions_release_the_returned_resource_without_touching_the_owner() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let owner = acquire(&host, "owner", json!({"handleId": "local"})).await;
    for local_id in ["", "local"] {
        host.acquire_resource(
            contribution(target("mount-a", 'a')),
            ResourceBindingId::from("rejected"),
            ResourceKind::from("fixture.resource"),
            StrictJsonValue(json!({"handleId": local_id})),
        )
        .await
        .unwrap_err();
    }
    let rejected_count = release_count(&host).await;
    host.release_resource(&owner).await.unwrap();
    let final_count = release_count(&host).await;
    host.stop_generation(generation).await.unwrap();
    assert_eq!(
        rejected_count, 2,
        "rejected acquisition leaked its release callback"
    );
    assert_eq!(final_count, 3);
}

#[tokio::test]
async fn release_keeps_the_plugin_resource_receiver() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let resource = acquire(
        &host,
        "receiver",
        json!({"handleId": "local", "requireReceiver": true}),
    )
    .await;
    let result = host.release_resource(&resource).await;
    host.stop_generation(generation).await.unwrap();
    result.unwrap();
}

#[tokio::test]
async fn rejected_resource_cleanup_failure_fails_the_generation() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let result = host
        .acquire_resource(
            contribution(target("mount-a", 'a')),
            ResourceBindingId::from("invalid"),
            ResourceKind::from("fixture.resource"),
            StrictJsonValue(json!({"handleId": "", "failRelease": true})),
        )
        .await;
    if matches!(result, Err(JavaScriptHostError::HostFailure { .. })) {
        wait_until_stopped(&host).await;
    } else {
        host.stop_generation(generation).await.unwrap();
    }
    assert!(matches!(
        result,
        Err(JavaScriptHostError::HostFailure { .. })
    ));
}

#[tokio::test]
async fn pending_invocations_block_only_their_own_mount_unload() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    load(&host, &temp, "mount-b", 'b').await;
    let invocation = host
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("wait_for_cancel"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    host.unload_mount(target("mount-b", 'b')).await.unwrap();
    assert!(matches!(
        host.unload_mount(target("mount-a", 'a')).await,
        Err(JavaScriptHostError::NotQuiescent { .. })
    ));
    host.cancel(invocation.request_id.clone()).await.unwrap();
    invocation.wait().await.unwrap_err();
    host.unload_mount(target("mount-a", 'a')).await.unwrap();
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn unload_releases_every_resource_before_acknowledging() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let log = temp.path().join("released.txt");
    acquire(&host, "first", json!({"releaseLog": log})).await;
    acquire(&host, "second", json!({"releaseLog": log})).await;
    host.unload_mount(target("mount-a", 'a')).await.unwrap();
    let releases = tokio::fs::read_to_string(log).await.unwrap_or_default();
    host.stop_generation(generation).await.unwrap();
    assert_eq!(releases, "first\nsecond\n");
}

#[tokio::test]
async fn stale_resource_release_cannot_touch_a_reloaded_mount() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let old = acquire(&host, "same-binding", json!({})).await;
    host.unload_mount(target("mount-a", 'a')).await.unwrap();
    assert_eq!(load(&host, &temp, "mount-a", 'a').await, generation);
    let new = acquire(&host, "same-binding", json!({})).await;
    let before = release_count(&host).await;
    let stale_result = host.release_resource(&old).await;
    let after = release_count(&host).await;
    let current_result = host.release_resource(&new).await;
    host.stop_generation(generation).await.unwrap();
    stale_result.unwrap();
    assert_eq!(after, before, "old lease released the new resource");
    current_result.unwrap();
    assert_ne!(old.handle_id, new.handle_id);
}

#[tokio::test]
async fn stale_resource_release_cannot_touch_a_reacquired_binding() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let old = acquire(&host, "same-binding", json!({})).await;
    host.release_resource(&old).await.unwrap();
    let new = acquire(&host, "same-binding", json!({})).await;
    let before = release_count(&host).await;
    let stale_result = host.release_resource(&old).await;
    let after = release_count(&host).await;
    let current_result = host.release_resource(&new).await;
    host.stop_generation(generation).await.unwrap();
    stale_result.unwrap();
    assert_eq!(after, before, "old lease released the reacquired binding");
    current_result.unwrap();
    assert_ne!(old.handle_id, new.handle_id);
}

#[tokio::test]
async fn unload_cleanup_failure_drains_other_resources_then_fails_generation() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    load(&host, &temp, "mount-a", 'a').await;
    let log = temp.path().join("released.txt");
    acquire(
        &host,
        "first",
        json!({"releaseLog": log, "failRelease": true}),
    )
    .await;
    acquire(&host, "second", json!({"releaseLog": log})).await;
    let result = host.unload_mount(target("mount-a", 'a')).await;
    if result.is_ok() {
        host.stop_generation(1).await.unwrap();
    } else {
        wait_until_stopped(&host).await;
    }
    assert!(matches!(
        result,
        Err(JavaScriptHostError::HostFailure { .. })
    ));
    assert_eq!(
        tokio::fs::read_to_string(log).await.unwrap(),
        "first\nsecond\n"
    );
    assert!(matches!(host.state(), JavaScriptHostState::Failed { .. }));
}
