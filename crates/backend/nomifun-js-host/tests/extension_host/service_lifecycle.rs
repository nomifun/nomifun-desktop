use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    PluginHostRequest, PluginHostResponseBody, PluginHostSuccess, SensitiveString,
};
use nomifun_js_host::{BoundHostServiceRequest, ExtensionHostServices};
use tokio::sync::{Notify, Semaphore};

struct ControlledServices {
    entered: Notify,
    release: Semaphore,
    dropped: AtomicUsize,
    completed: AtomicUsize,
    mounts: std::sync::Mutex<Vec<PluginMountRuntimeContext>>,
}

struct DropCounter<'a>(&'a AtomicUsize);

#[tokio::test]
async fn pending_activation_blocks_only_its_mount_commit_fence() {
    for activation in ["service", "service_then_reject"] {
        let (host, services, temp) = unloaded_setup(
            Duration::from_secs(5),
            fixture("../../assets/extension-host.mjs"),
        )
        .await;
        let mut mount = context(temp.path(), "mount-a", 'a');
        mount.config.value = StrictJsonValue(json!({"activation": activation}));
        let mut loading = Box::pin(host.load_mount(MountLoadDemand {
            module: module(mount.target.clone()).await,
            context: mount,
        }));
        tokio::select! {
            result = &mut loading => panic!("activation completed before service gate: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(5), services.entered.notified()) => result.unwrap(),
        }
        let pending = host
            .commit_fence_for_mount(&PluginMountId::from("mount-a"))
            .await;
        let unrelated = host
            .commit_fence_for_mount(&PluginMountId::from("mount-b"))
            .await;
        services.release.add_permits(1);
        let loaded = loading.await;
        let settled = host
            .commit_fence_for_mount(&PluginMountId::from("mount-a"))
            .await;
        host.stop_generation(1).await.unwrap();
        assert!(
            matches!(pending, Err(JavaScriptHostError::NotQuiescent { .. })),
            "pending {activation} activation was treated as empty: {pending:?}"
        );
        assert_eq!(unrelated.unwrap(), PluginHostCommitFence::NotResident);
        if activation == "service" {
            loaded.unwrap();
            assert!(matches!(
                settled,
                Err(JavaScriptHostError::NotQuiescent { .. })
            ));
        } else {
            loaded.unwrap_err();
            assert_eq!(settled.unwrap(), PluginHostCommitFence::NotResident);
        }
    }
}

impl Drop for DropCounter<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl ExtensionHostServices for ControlledServices {
    async fn handle(&self, request: BoundHostServiceRequest) -> PluginHostResponseBody {
        let _drop = DropCounter(&self.dropped);
        self.mounts.lock().unwrap().push(request.mount.clone());
        let PluginHostRequest::CredentialResolve { slot_key, .. } = request.envelope.request else {
            panic!("unexpected fixture service request");
        };
        assert_eq!(request.mount.mount_handle_id, "handle-mount-a");
        self.entered.notify_one();
        assert_ne!(slot_key.as_ref(), "panic", "fixture service panicked");
        self.release.acquire().await.unwrap().forget();
        self.completed.fetch_add(1, Ordering::SeqCst);
        PluginHostResponseBody::Success(PluginHostSuccess::CredentialResolved {
            slot_key,
            secret: SensitiveString("fixture-secret".into()),
        })
    }
}

async fn setup(
    timeout: Duration,
) -> (
    ExtensionHostSupervisor,
    Arc<ControlledServices>,
    TempDir,
    u64,
) {
    setup_with_host(timeout, fixture("../../assets/extension-host.mjs")).await
}

async fn setup_with_host(
    timeout: Duration,
    host: PathBuf,
) -> (
    ExtensionHostSupervisor,
    Arc<ControlledServices>,
    TempDir,
    u64,
) {
    let (supervisor, services, temp) = unloaded_setup(timeout, host).await;
    let generation = load(&supervisor, &temp, "mount-a", 'a').await;
    (supervisor, services, temp, generation)
}

async fn unloaded_setup(
    timeout: Duration,
    host: PathBuf,
) -> (ExtensionHostSupervisor, Arc<ControlledServices>, TempDir) {
    let services = Arc::new(ControlledServices {
        entered: Notify::new(),
        release: Semaphore::new(0),
        dropped: AtomicUsize::new(0),
        completed: AtomicUsize::new(0),
        mounts: std::sync::Mutex::new(Vec::new()),
    });
    let config = host_config(timeout, host.canonicalize().unwrap()).await;
    let supervisor = ExtensionHostSupervisor::with_services(config, services.clone()).unwrap();
    let temp = TempDir::new().unwrap();
    (supervisor, services, temp)
}

async fn start_service(supervisor: &ExtensionHostSupervisor, services: &ControlledServices) {
    supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("start_service"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), services.entered.notified())
        .await
        .expect("service should enter before the assertion");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activation_rejects_sdk_handles_not_reserved_by_a_mount_load() {
    let (host, services, temp) =
        unloaded_setup(Duration::from_secs(5), fixture("shutdown-service-host.mjs")).await;
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"unknownServiceHandle": true}));
    services.release.add_permits(1);
    let result = host
        .load_mount(MountLoadDemand {
            module: module(mount.target.clone()).await,
            context: mount,
        })
        .await;
    wait_until_stopped(&host).await;
    assert!(matches!(
        result,
        Err(JavaScriptHostError::HostFailure { .. })
    ));
    assert!(services.mounts.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_activation_with_a_hanging_service_is_bounded_and_cancelled() {
    let (host, services, temp) = unloaded_setup(
        Duration::from_millis(300),
        fixture("../../assets/extension-host.mjs"),
    )
    .await;
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"activation": "service_then_reject"}));
    let result = host
        .load_mount(MountLoadDemand {
            module: module(mount.target.clone()).await,
            context: mount,
        })
        .await;
    wait_until_stopped(&host).await;
    assert!(matches!(
        result,
        Err(JavaScriptHostError::HostFailure { .. })
    ));
    assert_eq!(services.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(services.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_activation_drains_services_before_releasing_its_context() {
    let (host, services, temp) = unloaded_setup(
        Duration::from_secs(5),
        fixture("../../assets/extension-host.mjs"),
    )
    .await;
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"activation": "service_then_reject"}));
    let demand = MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount.clone(),
    };
    let mut loading = Box::pin(host.load_mount(demand));
    tokio::select! {
        result = &mut loading => panic!("activation ended before its service completed: {result:?}"),
        _ = services.entered.notified() => {}
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut loading)
            .await
            .is_err()
    );
    services.release.add_permits(1);
    assert!(matches!(
        loading.await,
        Err(JavaScriptHostError::RequestFailed { .. })
    ));
    assert_eq!(*services.mounts.lock().unwrap(), vec![mount]);
    let generation = load(&host, &temp, "mount-a", 'a').await;
    assert_eq!(generation, 1);
    let stale = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("stale_service"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert!(stale.0["error"].is_string());
    host.stop_generation(generation).await.unwrap();
    assert_eq!(services.completed.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unloading_rejects_new_work_but_allows_cleanup_sdk_and_other_mounts() {
    let (host, services, temp) = unloaded_setup(
        Duration::from_secs(5),
        fixture("../../assets/extension-host.mjs"),
    )
    .await;
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"deactivationService": true}));
    let demand = MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    };
    let generation = host.load_mount(demand.clone()).await.unwrap();
    load(&host, &temp, "mount-b", 'b').await;
    let (unloading, ()) = tokio::join!(host.unload_mount(target("mount-a", 'a')), async {
        tokio::time::timeout(Duration::from_secs(2), services.entered.notified())
            .await
            .unwrap();
        let invoking = host
            .invoke(
                contribution(target("mount-a", 'a')),
                ActionId::from("echo"),
                StrictJsonValue(json!({})),
            )
            .await;
        let loading = host.load_mount(demand).await;
        let other = host
            .invoke(
                contribution(target("mount-b", 'b')),
                ActionId::from("echo"),
                StrictJsonValue(json!({})),
            )
            .await;
        services.release.add_permits(1);
        assert!(matches!(
            invoking,
            Err(JavaScriptHostError::NotQuiescent { .. })
        ));
        assert!(matches!(
            loading,
            Err(JavaScriptHostError::NotQuiescent { .. })
        ));
        other.unwrap();
    });
    unloading.unwrap();
    host.stop_generation(generation).await.unwrap();
    assert_eq!(services.completed.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_waits_for_sdk_work_started_by_cleanup() {
    let (host, services, temp) = unloaded_setup(
        Duration::from_secs(5),
        fixture("../../assets/extension-host.mjs"),
    )
    .await;
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value = StrictJsonValue(json!({"deactivationDetachedService": true}));
    let generation = host
        .load_mount(MountLoadDemand {
            module: module(mount.target.clone()).await,
            context: mount,
        })
        .await
        .unwrap();
    let mut unloading = Box::pin(host.unload_mount(target("mount-a", 'a')));
    tokio::select! {
        result = &mut unloading => panic!("unload ended before its cleanup service: {result:?}"),
        _ = services.entered.notified() => {}
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut unloading)
            .await
            .is_err()
    );
    services.release.add_permits(1);
    unloading.await.unwrap();
    assert_eq!(services.completed.load(Ordering::SeqCst), 1);
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activation_sdk_uses_the_exact_pending_mount_context() {
    let (host, services, temp) = unloaded_setup(
        Duration::from_secs(5),
        fixture("../../assets/extension-host.mjs"),
    )
    .await;
    let mut mount = context(temp.path(), "mount-a", 'a');
    mount.config.value =
        StrictJsonValue(json!({"activation": "service", "marker": "exact-context"}));
    services.release.add_permits(1);
    let generation = host
        .load_mount(MountLoadDemand {
            module: module(mount.target.clone()).await,
            context: mount.clone(),
        })
        .await
        .unwrap();
    let value = host
        .invoke(
            contribution(mount.target.clone()),
            ActionId::from("await_service"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    host.stop_generation(generation).await.unwrap();
    assert_eq!(value.0, json!("fixture-secret"));
    assert_eq!(*services.mounts.lock().unwrap(), vec![mount]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_rejects_outstanding_sdk_services() {
    let (host, services, _temp, generation) = setup(Duration::from_secs(5)).await;
    start_service(&host, &services).await;
    let unloading = host.unload_mount(target("mount-a", 'a')).await;
    services.release.add_permits(1);
    assert!(
        unloading.is_err(),
        "unload acknowledged while its SDK service was running"
    );
    host.invoke(
        contribution(target("mount-a", 'a')),
        ActionId::from("await_service"),
        StrictJsonValue(json!({})),
    )
    .await
    .unwrap();
    host.unload_mount(target("mount-a", 'a')).await.unwrap();
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_cannot_race_an_inflight_resource_acquisition() {
    let (host, services, _temp, generation) = setup(Duration::from_secs(5)).await;
    let (resource, unloading) = tokio::join!(
        host.acquire_resource(
            contribution(target("mount-a", 'a')),
            ResourceBindingId::from("waiting"),
            ResourceKind::from("fixture.resource"),
            StrictJsonValue(json!({"waitForService": true}))
        ),
        async {
            tokio::time::timeout(Duration::from_secs(2), services.entered.notified())
                .await
                .unwrap();
            let result = host.unload_mount(target("mount-a", 'a')).await;
            services.release.add_permits(1);
            result
        }
    );
    host.release_resource(&resource.unwrap()).await.unwrap();
    host.stop_generation(generation).await.unwrap();
    assert!(
        unloading.is_err(),
        "unload admitted an acquisition that had not finished"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_resource_releases_share_one_callback() {
    let (host, services, _temp, generation) = setup(Duration::from_secs(5)).await;
    let resource = host
        .acquire_resource(
            contribution(target("mount-a", 'a')),
            ResourceBindingId::from("release"),
            ResourceKind::from("fixture.resource"),
            StrictJsonValue(json!({"waitForReleaseService": true})),
        )
        .await
        .unwrap();
    let ((first, second), ()) = tokio::join!(
        async {
            tokio::join!(
                host.release_resource(&resource),
                host.release_resource(&resource)
            )
        },
        async {
            tokio::time::timeout(Duration::from_secs(2), services.entered.notified())
                .await
                .unwrap();
            services.release.add_permits(2);
        }
    );
    let count = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("resource_release_count"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    host.stop_generation(generation).await.unwrap();
    first.unwrap();
    second.unwrap();
    assert_eq!(count.0["count"], 1);
    assert_eq!(services.completed.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retired_sdk_cannot_bind_to_a_reloaded_mount_with_the_same_handle() {
    let (host, services, temp, generation) = setup(Duration::from_secs(5)).await;
    host.unload_mount(target("mount-a", 'a')).await.unwrap();
    assert_eq!(load(&host, &temp, "mount-a", 'a').await, generation);
    services.release.add_permits(1);
    let response = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("stale_service"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    host.stop_generation(generation).await.unwrap();
    assert!(
        response.0["error"].is_string(),
        "retired SDK was rebound to the new mount"
    );
    assert_eq!(services.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_requires_fire_and_forget_services_to_be_quiescent() {
    let (supervisor, services, _temp, generation) = setup(Duration::from_secs(5)).await;
    start_service(&supervisor, &services).await;
    assert!(matches!(
        supervisor.stop_generation(generation).await,
        Err(JavaScriptHostError::NotQuiescent { .. })
    ));
    services.release.add_permits(1);
    let response = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("await_service"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(response.0, json!("fixture-secret"));
    supervisor.stop_generation(generation).await.unwrap();
    assert_eq!(services.completed.load(Ordering::SeqCst), 1);
    assert_eq!(services.dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_cancels_services_before_publishing_failed_and_restarting() {
    let (supervisor, services, temp, generation) = setup(Duration::from_secs(5)).await;
    start_service(&supervisor, &services).await;
    supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("crash"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap_err();
    wait_until_stopped(&supervisor).await;
    assert!(matches!(
        supervisor.state(),
        JavaScriptHostState::Failed { .. }
    ));
    assert_eq!(
        services.dropped.load(Ordering::SeqCst),
        1,
        "old service outlived the generation"
    );
    services.release.add_permits(1);
    let next = load(&supervisor, &temp, "mount-a", 'a').await;
    assert!(next > generation);
    start_service(&supervisor, &services).await;
    supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("await_service"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(
        services.completed.load(Ordering::SeqCst),
        1,
        "only the new service may finish"
    );
    assert_eq!(services.dropped.load(Ordering::SeqCst), 2);
    supervisor.stop_generation(next).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fire_and_forget_service_timeout_fails_generation() {
    let (supervisor, services, _temp, _) = setup(Duration::from_millis(300)).await;
    start_service(&supervisor, &services).await;
    wait_until_stopped(&supervisor).await;
    let JavaScriptHostState::Failed { reason, .. } = supervisor.state() else {
        panic!("service timeout must fail the generation");
    };
    assert!(
        reason.contains("Host service") && reason.contains("timed out"),
        "{reason}"
    );
    assert_eq!(services.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(services.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_panic_fails_generation_and_cancels_siblings() {
    let (supervisor, services, _temp, _) = setup(Duration::from_secs(5)).await;
    start_service(&supervisor, &services).await;
    // The invocation may race failure publication, so its result is not the assertion.
    let _ = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("start_service"),
            StrictJsonValue(json!({"slot": "panic"})),
        )
        .await;
    wait_until_stopped(&supervisor).await;
    let JavaScriptHostState::Failed { reason, .. } = supervisor.state() else {
        panic!("service panic must fail the generation");
    };
    assert!(
        reason.contains("Host service") && reason.contains("panicked"),
        "{reason}"
    );
    assert!(
        !reason.contains("fixture service panicked"),
        "panic payload leaked into public state"
    );
    assert_eq!(services.dropped.load(Ordering::SeqCst), 2);
    assert_eq!(services.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_rejects_new_services_without_entering_the_handler() {
    let (supervisor, services, _temp, generation) =
        setup_with_host(Duration::from_secs(5), fixture("shutdown-service-host.mjs")).await;
    // If admitted, the handler would complete rather than block shutdown.
    services.release.add_permits(1);
    supervisor.stop_generation(generation).await.unwrap();
    assert_eq!(supervisor.state(), JavaScriptHostState::Stopped);
    assert_eq!(services.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(services.completed.load(Ordering::SeqCst), 0);
}
