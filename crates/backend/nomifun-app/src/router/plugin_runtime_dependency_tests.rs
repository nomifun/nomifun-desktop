use super::*;
use nomifun_agent_contracts::{PluginDependencyCall, PluginHostResponseBody, PluginHostSuccess};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

struct NestedCall {
    host: Arc<RuntimeBoundExtensionHost>,
    mount: MountLoadDemand,
    contribution: PluginHostContributionRef,
    started: Arc<Notify>,
    proceed: Arc<Notify>,
    dropped: Arc<AtomicUsize>,
}

struct DropCounter(Arc<AtomicUsize>);
impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl ExtensionHostDependencyCaller for NestedCall {
    async fn invoke(&self, call: PluginDependencyCall) -> PluginHostResponseBody {
        let _drop = DropCounter(self.dropped.clone());
        self.started.notify_one();
        self.proceed.notified().await;
        let result = self
            .host
            .invoke_demand(
                self.mount.clone(),
                self.contribution.clone(),
                call.action_id,
                call.input,
            )
            .await
            .expect("managed child should reuse the parent's runtime lease");
        PluginHostResponseBody::Success(PluginHostSuccess::Value(result))
    }
}

#[tokio::test]
async fn real_js_dependency_reuses_runtime_lease_behind_queued_writer_and_cleans_up_on_cancel() {
    run_runtime_lease_case(false).await;
}

#[tokio::test]
async fn real_js_context_dependency_reuses_runtime_lease_and_cleans_up_on_cancel() {
    run_runtime_lease_case(true).await;
}

async fn run_runtime_lease_case(context: bool) {
    for cancel in [false, true] {
        let (original, _, temp, mut demand) = setup(false).await;
        let old = original.state.lock().await.take().unwrap();
        let mut selection = nomifun_js_runtime::VersionedRuntimeSelection::empty();
        selection.selection.selected_runtime = Some(old.runtime.fingerprint.clone());
        selection.selected_executable_path = Some(old.runtime.executable_path);
        let authority = nomifun_js_runtime::authority_from_store(
            Arc::new(ReadOnlySelection(selection)),
            Arc::new(nomifun_js_runtime::SystemNodeRuntimeProbePort::default()),
        );
        let module =
            nomifun_js_host::materialize_bundled_extension_host(temp.path().join("host")).unwrap();
        let host = RuntimeBoundExtensionHost::new(authority.clone(), module).unwrap();
        let source = include_bytes!("../../tests/fixtures/runtime-dependency-tool.mjs");
        tokio::fs::write(&demand.module.main_mjs, source)
            .await
            .unwrap();
        demand.module.module_digest = DigestHex::from(format!("{:x}", Sha256::digest(source)));
        let contribution = PluginHostContributionRef {
            target: demand.context.target.clone(),
            capability: nomifun_agent_contracts::CapabilityRef {
                id: "fixture.parent".into(),
            },
            contribution_id: "fixture.dependency".into(),
            contract_digest: DigestHex::from("e".repeat(64)),
        };
        let started = Arc::new(Notify::new());
        let proceed = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicUsize::new(0));
        let callback = Arc::new(NestedCall {
            host: host.clone(),
            mount: demand.clone(),
            contribution: contribution.clone(),
            started: started.clone(),
            proceed: proceed.clone(),
            dropped: dropped.clone(),
        });
        let parent = tokio::spawn({
            let host = host.clone();
            async move {
                if context {
                    return host
                        .contribute_context_demand(
                            demand,
                            contribution,
                            "schema://fixture/context".into(),
                            Default::default(),
                            Some(callback),
                        )
                        .await;
                }
                host.invoke_with_dependencies(
                    demand,
                    contribution,
                    "relay".into(),
                    StrictJsonValue(json!({"value":23})),
                    callback,
                )
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        let mut writer = Box::pin(authority.acquire_switch_fence());
        assert!(
            matches!(
                std::future::poll_fn(|cx| std::task::Poll::Ready(writer.as_mut().poll(cx))).await,
                std::task::Poll::Pending
            ),
            "parent must pin the selected runtime"
        );
        if cancel {
            parent.abort();
            assert!(parent.await.unwrap_err().is_cancelled());
        } else {
            proceed.notify_one();
            let result = tokio::time::timeout(Duration::from_secs(3), parent)
                .await
                .expect("nested demand deadlocked behind the queued writer")
                .unwrap()
                .unwrap();
            assert_eq!(result.0, json!({"nested":true,"input":{"value":23}}));
        }
        let fence = tokio::time::timeout(Duration::from_secs(3), writer)
            .await
            .expect("parent termination leaked the runtime read lease");
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        // No second supervisor or outstanding callback may keep the Host busy.
        host.stop_for_runtime_switch().await.unwrap();
        drop(fence);
    }
}

#[tokio::test]
async fn runtime_dependency_scope_cannot_be_borrowed_by_another_host_or_detached_task() {
    let (original, supervisor, _temp, _) = setup(false).await;
    let bound = original.state.lock().await.take().unwrap();
    let mut selection = nomifun_js_runtime::VersionedRuntimeSelection::empty();
    selection.selection.selected_runtime = Some(bound.runtime.fingerprint);
    selection.selected_executable_path = Some(bound.runtime.executable_path);
    let authority = nomifun_js_runtime::authority_from_store(
        Arc::new(ReadOnlySelection(selection)),
        Arc::new(nomifun_js_runtime::SystemNodeRuntimeProbePort::default()),
    );
    let lease = authority
        .acquire_use(JavaScriptWorkKind::SharedExtensionHost)
        .await
        .unwrap();
    let other =
        RuntimeBoundExtensionHost::new(Arc::new(UnusedRuntime), original.host_module.clone())
            .unwrap();
    DEPENDENCY_RUNTIME
        .scope(
            DependencyRuntime {
                identity: original.identity.clone(),
                lease,
                supervisor,
            },
            async {
                assert!(original.demand_host().await.is_ok());
                assert!(
                    other.demand_host().await.is_err(),
                    "another Host borrowed the parent's lease"
                );
                assert!(
                    tokio::spawn({
                        let original = original.clone();
                        async move { original.demand_host().await.is_err() }
                    })
                    .await
                    .unwrap(),
                    "detached tasks must obtain their own runtime admission"
                );
            },
        )
        .await;
    tokio::time::timeout(Duration::from_secs(1), authority.acquire_switch_fence())
        .await
        .unwrap();
}
