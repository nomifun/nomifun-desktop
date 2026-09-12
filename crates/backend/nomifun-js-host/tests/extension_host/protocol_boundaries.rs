use super::*;
use async_trait::async_trait;
use nomifun_agent_contracts::PluginHostResponseBody;
use nomifun_js_host::{BoundHostServiceRequest, ExtensionHostServices};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct CountingServices(AtomicUsize);

#[async_trait]
impl ExtensionHostServices for CountingServices {
    async fn handle(&self, _request: BoundHostServiceRequest) -> PluginHostResponseBody {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
}

async fn setup(
    mode: &str,
) -> (
    ExtensionHostSupervisor,
    Arc<CountingServices>,
    TempDir,
    MountLoadDemand,
) {
    let mut config = host_config(
        Duration::from_millis(400),
        fixture("protocol-host.mjs").canonicalize().unwrap(),
    )
    .await;
    if mode == "service-flood" {
        config.limits.max_service_requests = 2;
    }
    if mode == "cancel-hang" {
        config.limits.max_pending_requests = 1;
        config.limits.request_timeout = Duration::from_secs(2);
    }
    let services = Arc::new(CountingServices::default());
    let host = ExtensionHostSupervisor::with_services(config, services.clone()).unwrap();
    let temp = TempDir::new().unwrap();
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"mode": mode}));
    let demand = MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    };
    (host, services, temp, demand)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_role_must_match_the_active_host_before_entering_handler() {
    let (host, services, _temp, demand) = setup("wrong-role").await;
    host.load_mount(demand).await.unwrap();
    wait_until_stopped(&host).await;
    assert_eq!(
        services.0.load(Ordering::SeqCst),
        0,
        "wrong-role request entered service"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_duplicate_service_ids_cannot_execute_twice() {
    let (host, services, _temp, demand) = setup("duplicate-service").await;
    host.load_mount(demand).await.unwrap();
    wait_until_stopped(&host).await;
    assert!(
        services.0.load(Ordering::SeqCst) <= 1,
        "duplicate correlation executed service twice"
    );
    assert!(
        matches!(host.state(), JavaScriptHostState::Failed { reason, .. } if reason.contains("duplicate"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inflight_service_limit_rejects_excess_before_handler_entry() {
    let (host, services, _temp, demand) = setup("service-flood").await;
    host.load_mount(demand).await.unwrap();
    wait_until_stopped(&host).await;
    let entered = services.0.load(Ordering::SeqCst);
    assert!(
        entered <= 2,
        "excess service requests entered handlers: {entered}"
    );
    assert!(
        matches!(host.state(), JavaScriptHostState::Failed { reason, .. }
        if reason.contains("capacity"))
    );
}

#[tokio::test]
async fn inflight_cancellation_slot_is_bounded_and_independent_of_work() {
    let (host, _services, _temp, demand) = setup("cancel-hang").await;
    let contribution = contribution(demand.context.target.clone());
    host.load_mount(demand).await.unwrap();
    let first = host.cancel("first-target".into());
    let overflow = async {
        // A real response proves the first cancellation reached the peer,
        // while ordinary work must still be admitted into its separate slot.
        loop {
            let value = host
                .invoke(
                    contribution.clone(),
                    ActionId::from("echo"),
                    StrictJsonValue(json!({})),
                )
                .await
                .unwrap();
            if value.0["cancellations"] == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        host.cancel("second-target".into()).await
    };
    let (first, overflow) = tokio::join!(first, overflow);
    wait_until_stopped(&host).await;
    assert_eq!(overflow.unwrap_err(), JavaScriptHostError::QueueFull);
    assert!(matches!(
        first,
        Err(JavaScriptHostError::HostFailure { .. })
    ));
}

#[tokio::test]
async fn invalid_response_delivers_host_failure_instead_of_dropping_waiter() {
    let (host, _services, _temp, demand) = setup("invalid-response").await;
    let result = host.load_mount(demand).await;
    wait_until_stopped(&host).await;
    assert!(
        matches!(
            result,
            Err(JavaScriptHostError::HostFailure { generation: 1, .. })
        ),
        "waiter lost validated failure: {result:?}"
    );
}

#[tokio::test]
async fn public_protocol_failures_do_not_quote_peer_values() {
    let mut leaked = Vec::new();
    for mode in [
        "unknown-response",
        "invalid-version",
        "service-timeout",
        "shutdown-rejected",
    ] {
        let (host, _services, _temp, demand) = setup(mode).await;
        let result = host.load_mount(demand).await;
        if mode == "shutdown-rejected" {
            let _ = host.stop_generation(result.unwrap()).await;
        }
        wait_until_stopped(&host).await;
        let JavaScriptHostState::Failed { reason, .. } = host.state() else {
            panic!("expected protocol failure")
        };
        if reason.contains("fixture-wire-secret") {
            leaked.push((mode, reason));
        }
    }
    assert!(
        leaked.is_empty(),
        "untrusted wire values leaked: {leaked:?}"
    );
}

#[tokio::test]
async fn invalid_shutdown_response_also_delivers_the_generation_failure() {
    let (host, _services, _temp, demand) = setup("invalid-stop").await;
    let generation = host.load_mount(demand).await.unwrap();
    let result = host.stop_generation(generation).await;
    wait_until_stopped(&host).await;
    assert!(
        matches!(
            result,
            Err(JavaScriptHostError::HostFailure { generation: 1, .. })
        ),
        "stop waiter lost failure: {result:?}"
    );
}
