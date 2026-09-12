use super::*;
use async_trait::async_trait;
use nomifun_agent_contracts::{
    PluginHostRequest, PluginHostResponseBody, PluginHostSuccess, SensitiveString,
};
use nomifun_js_host::{BoundHostServiceRequest, ExtensionHostServices};
use std::sync::Arc;

const LARGE_PAYLOAD: usize = 4 * 1024 * 1024;

#[tokio::test]
async fn valid_large_frames_round_trip_through_the_real_host() {
    let mut config = host_config(
        Duration::from_secs(5),
        fixture("../../assets/extension-host.mjs")
            .canonicalize()
            .unwrap(),
    )
    .await;
    config.limits.max_frame_bytes = 8 * 1024 * 1024;
    let host = ExtensionHostSupervisor::new(config).unwrap();
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let payload = "x".repeat(LARGE_PAYLOAD);
    let result = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!(&payload)),
        )
        .await;
    host.stop_generation(generation).await.unwrap();
    assert_eq!(result.unwrap().0["input"], payload);
}

async fn blocked_config(timeout: Duration) -> JavaScriptHostConfig {
    let mut config = host_config(
        timeout,
        fixture("backpressure-host.mjs").canonicalize().unwrap(),
    )
    .await;
    config.limits.max_frame_bytes = 8 * 1024 * 1024;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_request_write_does_not_block_watchdog() {
    let host =
        ExtensionHostSupervisor::new(blocked_config(Duration::from_millis(300)).await).unwrap();
    let temp = TempDir::new().unwrap();
    load(&host, &temp, "mount-a", 'a').await;
    let result = tokio::time::timeout(
        Duration::from_millis(1500),
        host.invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!("x".repeat(LARGE_PAYLOAD))),
        ),
    )
    .await;
    // Also reap the bounded fallback on the old implementation before asserting.
    wait_until_stopped(&host).await;
    assert!(
        matches!(result, Ok(Err(JavaScriptHostError::HostFailure { .. }))),
        "write blocked supervision: {result:?}"
    );
    assert!(
        matches!(host.subscribe_state().borrow().clone(), JavaScriptHostState::Failed { reason, .. } if reason.contains("timed out"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_request_write_does_not_block_stop_admission() {
    let host = ExtensionHostSupervisor::new(blocked_config(Duration::from_secs(2)).await).unwrap();
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let pending = host
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!("x".repeat(LARGE_PAYLOAD))),
        )
        .await
        .unwrap();
    let stopped =
        tokio::time::timeout(Duration::from_millis(500), host.stop_generation(generation)).await;
    wait_until_stopped(&host).await;
    assert!(
        matches!(stopped, Ok(Err(JavaScriptHostError::NotQuiescent { .. }))),
        "stop admission blocked: {stopped:?}"
    );
    assert!(pending.wait().await.is_err());
}

struct LargeService;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbound_queue_overflow_rejects_only_the_unsent_request() {
    let mut config = blocked_config(Duration::from_secs(2)).await;
    config.limits.command_queue_capacity = 1;
    let host = ExtensionHostSupervisor::new(config).unwrap();
    let temp = TempDir::new().unwrap();
    load(&host, &temp, "mount-a", 'a').await;
    let pending = host
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!("x".repeat(LARGE_PAYLOAD))),
        )
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        host.invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({})),
        ),
    )
    .await;
    let still_running = host.process_count();
    wait_until_stopped(&host).await;
    assert!(
        matches!(result, Ok(Err(JavaScriptHostError::QueueFull))),
        "overflow was not rejected locally: {result:?}"
    );
    assert_eq!(still_running, 1);
    assert!(pending.wait().await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_service_response_fails_generation_without_exposing_secret() {
    let mut config = blocked_config(Duration::from_secs(2)).await;
    config.limits.max_frame_bytes = 2048;
    let host = ExtensionHostSupervisor::with_services(config, Arc::new(LargeService)).unwrap();
    let temp = TempDir::new().unwrap();
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"service": true}));
    host.load_mount(MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    })
    .await
    .unwrap();
    wait_until_stopped(&host).await;
    let JavaScriptHostState::Failed { reason, .. } = host.subscribe_state().borrow().clone() else {
        panic!("must fail closed")
    };
    assert!(reason.contains("exceeds 2048 bytes"), "{reason}");
    assert!(!reason.contains(&"x".repeat(32)));
}

#[async_trait]
impl ExtensionHostServices for LargeService {
    async fn handle(&self, request: BoundHostServiceRequest) -> PluginHostResponseBody {
        let PluginHostRequest::CredentialResolve { slot_key, .. } = request.envelope.request else {
            panic!("unexpected service")
        };
        PluginHostResponseBody::Success(PluginHostSuccess::CredentialResolved {
            slot_key,
            secret: SensitiveString("x".repeat(LARGE_PAYLOAD)),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_independent_service_response_has_a_write_deadline() {
    let host = ExtensionHostSupervisor::with_services(
        blocked_config(Duration::from_millis(300)).await,
        Arc::new(LargeService),
    )
    .unwrap();
    let temp = TempDir::new().unwrap();
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"service": true}));
    host.load_mount(MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    })
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_millis(1500), wait_until_stopped(&host)).await;
    wait_until_stopped(&host).await;
    assert!(result.is_ok(), "service response blocked supervision");
    assert!(
        matches!(host.subscribe_state().borrow().clone(), JavaScriptHostState::Failed { reason, .. } if reason.contains("timed out"))
    );
}

#[tokio::test]
async fn oversized_outbound_request_is_rejected_without_killing_host() {
    let mut config = host_config(
        Duration::from_secs(2),
        fixture("../../assets/extension-host.mjs")
            .canonicalize()
            .unwrap(),
    )
    .await;
    config.limits.max_frame_bytes = 2048;
    let host = ExtensionHostSupervisor::new(config).unwrap();
    let temp = TempDir::new().unwrap();
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let result = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!("x".repeat(8192))),
        )
        .await;
    let next = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({})),
        )
        .await;
    if next.is_ok() {
        host.stop_generation(generation).await.unwrap();
    } else {
        wait_until_stopped(&host).await;
    }
    assert!(
        matches!(result, Err(JavaScriptHostError::Contract(_))),
        "oversized request reached peer: {result:?}"
    );
    assert!(
        next.is_ok(),
        "local oversize must not retire a healthy Host"
    );
}
