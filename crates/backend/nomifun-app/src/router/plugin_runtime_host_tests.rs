use super::*;
use nomifun_agent_contracts::{
    DigestHex, PackageId, PackageRef, PluginHostTargetLock, PluginMountRuntimeContext,
    PluginStateHandleDescriptor, PluginStateMethod, ValidatedPluginConfig, VersionString,
};
use nomifun_js_host::ImmutablePluginModule;
use nomifun_js_runtime::{JavaScriptRuntimeError, NodeProbeCandidate, NodeRuntimeResolver};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, time::Duration};
use tempfile::TempDir;

struct UnusedRuntime;

struct ReadOnlySelection(nomifun_js_runtime::VersionedRuntimeSelection);

#[async_trait]
impl nomifun_js_runtime::RuntimeSelectionStore for ReadOnlySelection {
    async fn load(
        &self,
    ) -> Result<
        nomifun_js_runtime::VersionedRuntimeSelection,
        nomifun_js_runtime::RuntimeSelectionStoreError,
    > {
        Ok(self.0.clone())
    }

    async fn save_cas(
        &self,
        _: u64,
        _: &nomifun_agent_contracts::RuntimeSelectionRecord,
        _: Option<&std::path::Path>,
        _: Option<&std::path::Path>,
        _: i64,
    ) -> Result<
        nomifun_js_runtime::VersionedRuntimeSelection,
        nomifun_js_runtime::RuntimeSelectionStoreError,
    > {
        panic!("read-only fixture must not mutate Runtime selection")
    }
}

#[tokio::test]
async fn auto_apply_reuses_its_lease_when_a_runtime_writer_is_queued() {
    let (original, supervisor, temp, demand) = setup(false).await;
    supervisor.load_mount(demand).await.unwrap();
    let bound = original.state.lock().await.take().unwrap();
    let mut selection = nomifun_js_runtime::VersionedRuntimeSelection::empty();
    selection.selection.selected_runtime = Some(bound.runtime.fingerprint.clone());
    selection.selected_executable_path = Some(bound.runtime.executable_path.clone());
    let authority = nomifun_js_runtime::authority_from_store(
        Arc::new(ReadOnlySelection(selection)),
        Arc::new(nomifun_js_runtime::SystemNodeRuntimeProbePort::default()),
    );
    let binding =
        RuntimeBoundExtensionHost::new(authority.clone(), original.host_module.clone()).unwrap();
    *binding.state.lock().await = Some(bound);
    let lease = authority
        .acquire_use(JavaScriptWorkKind::SharedExtensionHost)
        .await
        .unwrap();
    let mut writer = Box::pin(authority.acquire_switch_fence());
    assert!(matches!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(writer.as_mut().poll(cx))).await,
        std::task::Poll::Pending
    ));
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        binding.try_auto_apply_fence_for_mount(&PluginMountId::from("absent"), &lease),
    )
    .await;
    drop(lease);
    let fence = tokio::time::timeout(Duration::from_secs(1), writer)
        .await
        .unwrap();
    tokio::fs::write(temp.path().join("finish"), b"go")
        .await
        .unwrap();
    binding.stop_for_runtime_switch().await.unwrap();
    drop(fence);
    assert!(
        matches!(result, Ok(Ok(Some(PluginHostCommitFence::NotResident)))),
        "nested Runtime read was blocked behind a writer: {result:?}"
    );
}

#[async_trait]
impl CommittedRuntimeProvider for UnusedRuntime {
    async fn acquire_use(
        &self,
        _: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        Err(JavaScriptRuntimeError::SwitchNotCovered(
            "fixture has no lease".into(),
        ))
    }

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        Ok(None)
    }
}

async fn setup(
    invalid_hello: bool,
) -> (
    Arc<RuntimeBoundExtensionHost>,
    Arc<ExtensionHostSupervisor>,
    TempDir,
    MountLoadDemand,
) {
    let temp = TempDir::new().unwrap();
    let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join(if cfg!(windows) { "node.exe" } else { "node" }))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let probe = NodeRuntimeResolver::default()
        .probe(&NodeProbeCandidate::new(
            nomifun_agent_contracts::NodeRuntimeSourceKind::ProcessPath,
            executable.clone(),
        ))
        .await;
    let runtime = ResolvedNodeRuntime {
        executable_path: executable.clone(),
        fingerprint: probe.fingerprint.unwrap(),
    };
    let host_module = temp.path().join("host.mjs");
    let host_source: &[u8] = if invalid_hello {
        b"console.log('{}');setTimeout(() => {}, 10000);"
    } else {
        include_bytes!("../../tests/fixtures/runtime-stop-host.mjs")
    };
    tokio::fs::write(&host_module, host_source).await.unwrap();
    let supervisor = Arc::new(
        ExtensionHostSupervisor::new(JavaScriptHostConfig::for_host_module(
            executable,
            runtime.fingerprint.clone(),
            host_module.clone(),
        ))
        .unwrap(),
    );
    let binding = RuntimeBoundExtensionHost::new(Arc::new(UnusedRuntime), host_module).unwrap();
    *binding.state.lock().await = Some(BoundExtensionHost {
        runtime,
        supervisor: supervisor.clone(),
    });
    let target = PluginHostTargetLock {
        mount_id: PluginMountId::from("fixture-mount"),
        package: PackageRef {
            id: PackageId::from("fixture.package"),
            version: VersionString::from("1.0.0"),
        },
        artifact_digest: DigestHex::from("a".repeat(64)),
        manifest_digest: DigestHex::from("b".repeat(64)),
    };
    let context = PluginMountRuntimeContext {
        target: target.clone(),
        mount_handle_id: "fixture-mount-handle".into(),
        config: ValidatedPluginConfig {
            schema_digest: DigestHex::from("c".repeat(64)),
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        credential_bindings: Vec::new(),
        state: PluginStateHandleDescriptor {
            package_id: target.package.id.clone(),
            mount_id: target.mount_id.clone(),
            methods: PluginStateMethod::REQUIRED
                .into_iter()
                .collect::<BTreeSet<_>>(),
        },
        data_dir: temp.path().display().to_string(),
    };
    let module_path = temp.path().join("main.mjs");
    let module = b"export async function activate() { return {}; }";
    tokio::fs::write(&module_path, module).await.unwrap();
    let demand = MountLoadDemand {
        context,
        module: ImmutablePluginModule::new(
            module_path,
            DigestHex::from(format!("{:x}", Sha256::digest(module))),
            target,
        ),
    };
    (binding, supervisor, temp, demand)
}

#[tokio::test]
async fn cancelled_runtime_stop_retains_binding_and_never_exposes_a_false_empty_window() {
    let (binding, supervisor, temp, demand) = setup(false).await;
    let mount_id = demand.context.target.mount_id.clone();
    supervisor.load_mount(demand).await.unwrap();
    let mut stopping = Box::pin(binding.stop_for_runtime_switch());
    let ready = async {
        while !temp.path().join("stopping").is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::select! {
        result = &mut stopping => panic!("stop completed before fixture gate: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(5), ready) => result.unwrap(),
    }
    let early_fence = tokio::time::timeout(
        Duration::from_millis(30),
        binding.commit_fence_for_mount(&mount_id),
    )
    .await;
    drop(stopping);
    let retained = binding.state.lock().await.is_some();
    tokio::fs::write(temp.path().join("finish"), b"go")
        .await
        .unwrap();
    let mut state = supervisor.subscribe_state();
    tokio::time::timeout(Duration::from_secs(5), async {
        while matches!(*state.borrow(), JavaScriptHostState::Running { .. }) {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    // Retry must retire the same binding after the Actor completed cleanup.
    binding.stop_for_runtime_switch().await.unwrap();
    assert!(binding.state.lock().await.is_none());
    assert!(
        retained && early_fence.is_err(),
        "cancelled stop lost binding={}; premature fence={early_fence:?}",
        !retained
    );
}

#[tokio::test]
async fn failed_host_with_completed_cleanup_can_be_retired_for_runtime_switch() {
    let (binding, supervisor, _temp, demand) = setup(true).await;
    assert!(supervisor.load_mount(demand).await.is_err());
    assert!(matches!(
        supervisor.state(),
        JavaScriptHostState::Failed { .. }
    ));
    let result = binding.stop_for_runtime_switch().await;
    assert!(
        result.is_ok(),
        "completed cleanup was treated as permanent failure: {result:?}"
    );
    assert!(binding.state.lock().await.is_none());
}
