use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nomifun_agent_contracts::{
    ActionId, CanonicalSchemaRef, CapabilityId, CapabilityRef, ContributionId,
    CredentialSlotBinding, DigestHex, PackageId, PackageRef, PluginHostCommitFence,
    PluginHostContributionRef, PluginHostTargetLock, PluginMountId,
    PluginMountRuntimeContext, PluginStateHandleDescriptor, PluginStateMethod,
    JavaScriptHostKind, ResourceBindingId, ResourceKind, StrictJsonValue, ValidatedPluginConfig,
    VersionString,
};
use nomifun_js_host::{
    ExtensionHostSupervisor, ImmutablePluginModule, JavaScriptHostConfig,
    JavaScriptHostError, JavaScriptHostLimits, JavaScriptHostState,
    MountLoadDemand, materialize_bundled_extension_host,
};
use nomifun_js_runtime::{NodeProbeCandidate, NodeRuntimeResolver};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(path)
}

async fn runtime() -> (PathBuf, nomifun_agent_contracts::NodeRuntimeFingerprint) {
    let executable = which_node();
    let result = NodeRuntimeResolver::default()
        .probe(&NodeProbeCandidate::new(
            nomifun_agent_contracts::NodeRuntimeSourceKind::ProcessPath,
            executable.clone(),
        ))
        .await;
    (
        executable,
        result
            .fingerprint
            .unwrap_or_else(|| panic!("Node probe failed: {:?}", result.error_code)),
    )
}

fn which_node() -> PathBuf {
    let path = std::env::var_os("PATH").expect("PATH is available");
    std::env::split_paths(&path)
        .flat_map(|directory| {
            #[cfg(windows)]
            let names = ["node.exe", "node"];
            #[cfg(not(windows))]
            let names = ["node", "node"];
            names.map(move |name| directory.join(name))
        })
        .find(|candidate| candidate.is_file())
        .expect("Node 22+ is required for JavaScript Host tests")
        .canonicalize()
        .expect("Node path is canonical")
}

async fn supervisor(
    request_timeout: Duration,
) -> ExtensionHostSupervisor {
    supervisor_with_host(
        request_timeout,
        fixture("../../assets/extension-host.mjs")
            .canonicalize()
            .expect("bundled Host asset exists"),
    )
    .await
}

async fn supervisor_with_host(
    request_timeout: Duration,
    host_module: PathBuf,
) -> ExtensionHostSupervisor {
    let (node_executable, runtime) = runtime().await;
    ExtensionHostSupervisor::new(JavaScriptHostConfig {
        node_executable,
        runtime,
        host_module,
        limits: JavaScriptHostLimits {
            hello_timeout: Duration::from_secs(5),
            request_timeout,
            shutdown_timeout: Duration::from_secs(5),
            max_frame_bytes: 1024 * 1024,
            command_queue_capacity: 32,
        },
    })
    .expect("Host configuration is valid")
}

async fn candidate_test_supervisor(
    request_timeout: Duration,
) -> ExtensionHostSupervisor {
    let (node_executable, runtime) = runtime().await;
    ExtensionHostSupervisor::candidate_test(JavaScriptHostConfig {
        node_executable,
        runtime,
        host_module: fixture("../../assets/extension-host.mjs")
            .canonicalize()
            .expect("bundled Host asset exists"),
        limits: JavaScriptHostLimits {
            hello_timeout: Duration::from_secs(5),
            request_timeout,
            shutdown_timeout: Duration::from_secs(5),
            max_frame_bytes: 1024 * 1024,
            command_queue_capacity: 32,
        },
    })
    .expect("Candidate Test Host configuration is valid")
}

async fn module(target: PluginHostTargetLock) -> ImmutablePluginModule {
    let path = fixture("main.mjs")
        .canonicalize()
        .expect("fixture module exists");
    let bytes = tokio::fs::read(&path).await.unwrap();
    ImmutablePluginModule::new(
        path,
        DigestHex::from(hex::encode(Sha256::digest(bytes))),
        target,
    )
}

fn target(mount_id: &str, digest: char) -> PluginHostTargetLock {
    let manifest_digest = match digest {
        'f' => 'e',
        value => char::from_u32(u32::from(value) + 1)
            .expect("fixture digest character increments"),
    };
    PluginHostTargetLock {
        mount_id: PluginMountId::from(mount_id),
        package: PackageRef {
            id: PackageId::from(format!("package.{mount_id}")),
            version: VersionString::from("1.0.0"),
        },
        artifact_digest: DigestHex::from(digest.to_string().repeat(64)),
        manifest_digest: DigestHex::from(manifest_digest.to_string().repeat(64)),
    }
}

fn context(root: &Path, mount_id: &str, digest: char) -> PluginMountRuntimeContext {
    let target = target(mount_id, digest);
    PluginMountRuntimeContext {
        target: target.clone(),
        mount_handle_id: format!("handle-{mount_id}"),
        config: ValidatedPluginConfig {
            schema_digest: DigestHex::from("d".repeat(64)),
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        credential_bindings: Vec::<CredentialSlotBinding>::new(),
        state: PluginStateHandleDescriptor {
            package_id: target.package.id.clone(),
            mount_id: target.mount_id.clone(),
            methods: PluginStateMethod::REQUIRED.into_iter().collect::<BTreeSet<_>>(),
        },
        data_dir: root.join(mount_id).display().to_string(),
    }
}

fn contribution(
    target: PluginHostTargetLock,
) -> PluginHostContributionRef {
    PluginHostContributionRef {
        contribution_id: ContributionId::from(format!(
            "contribution.{}",
            target.mount_id.as_ref()
        )),
        capability: CapabilityRef {
            id: CapabilityId::from("fixture.echo"),
            version: VersionString::from("1.0.0"),
        },
        contract_digest: DigestHex::from("e".repeat(64)),
        target,
    }
}

async fn load(
    supervisor: &ExtensionHostSupervisor,
    temp: &TempDir,
    mount_id: &str,
    digest: char,
) -> u64 {
    let context = context(temp.path(), mount_id, digest);
    let module_target = context.target.clone();
    tokio::fs::create_dir_all(&context.data_dir).await.unwrap();
    supervisor
        .load_mount(MountLoadDemand {
            context,
            module: module(module_target).await,
        })
        .await
        .unwrap()
}

async fn wait_until_stopped(supervisor: &ExtensionHostSupervisor) {
    let mut state = supervisor.subscribe_state();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !matches!(
                state.borrow().clone(),
                JavaScriptHostState::Running { .. }
            ) {
                return;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("Host generation should stop");
}

#[test]
fn bundled_host_is_materialized_as_an_immutable_digest_named_file() {
    let temp = TempDir::new().unwrap();
    let first = materialize_bundled_extension_host(temp.path()).unwrap();
    let second = materialize_bundled_extension_host(temp.path()).unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                value.starts_with("extension-host-")
                    && value.ends_with(".mjs")
            })
    );
    assert_eq!(
        std::fs::read(first).unwrap(),
        include_bytes!("../assets/extension-host.mjs")
    );
}

#[tokio::test]
async fn demand_zero_then_first_mount_starts_and_two_mounts_join_lazily() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();

    assert_eq!(supervisor.process_count(), 0);
    assert_eq!(supervisor.state(), JavaScriptHostState::Stopped);

    let generation = load(&supervisor, &temp, "mount-a", 'a').await;
    assert_eq!(generation, 1);
    assert_eq!(supervisor.process_count(), 1);
    assert_eq!(
        load(&supervisor, &temp, "mount-a", 'a').await,
        generation,
        "an exact resident Mount load is idempotent"
    );

    let first = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({"value": 1})),
        )
        .await
        .unwrap();
    assert_eq!(first.0["input"], json!({"value": 1}));
    assert_eq!(first.0["mount_id"], "mount-a");

    assert_eq!(load(&supervisor, &temp, "mount-b", 'b').await, generation);
    let second = supervisor
        .invoke(
            contribution(target("mount-b", 'b')),
            ActionId::from("echo"),
            StrictJsonValue(json!({"value": 2})),
        )
        .await
        .unwrap();
    assert_eq!(second.0["mount_id"], "mount-b");

    let fence = supervisor.stop_generation(generation).await.unwrap();
    assert!(matches!(
        fence,
        PluginHostCommitFence::ResidentFenced {
            host_generation: 1,
            ..
        }
    ));
    assert_eq!(supervisor.process_count(), 0);
}

#[tokio::test]
async fn candidate_test_role_uses_an_isolated_generation_and_exact_protocol() {
    let shared = supervisor(Duration::from_secs(2)).await;
    let candidate = candidate_test_supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();

    assert_eq!(shared.host_kind(), JavaScriptHostKind::SharedExtension);
    assert_eq!(candidate.host_kind(), JavaScriptHostKind::CandidateTest);
    let shared_generation = load(&shared, &temp, "mount-shared", 'a').await;
    let candidate_generation = load(&candidate, &temp, "mount-candidate", 'b').await;
    let JavaScriptHostState::Running {
        process_id: shared_process,
        ..
    } = shared.state()
    else {
        panic!("shared Host must be running");
    };
    let JavaScriptHostState::Running {
        process_id: candidate_process,
        ..
    } = candidate.state()
    else {
        panic!("Candidate Test Host must be running");
    };
    assert_ne!(shared_process, candidate_process);

    let value = candidate
        .invoke(
            contribution(target("mount-candidate", 'b')),
            ActionId::from("echo"),
            StrictJsonValue(json!({"candidate": true})),
        )
        .await
        .unwrap();
    assert_eq!(value.0["input"]["candidate"], true);

    candidate.stop_generation(candidate_generation).await.unwrap();
    assert_eq!(candidate.process_count(), 0);
    assert_eq!(shared.process_count(), 1);
    shared.stop_generation(shared_generation).await.unwrap();
}

#[tokio::test]
async fn mount_handle_identity_is_unique_within_generation() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&supervisor, &temp, "mount-a", 'a').await;
    let mut duplicate = context(temp.path(), "mount-b", 'b');
    duplicate.mount_handle_id = "handle-mount-a".into();
    tokio::fs::create_dir_all(&duplicate.data_dir).await.unwrap();
    let error = supervisor
        .load_mount(MountLoadDemand {
            module: module(duplicate.target.clone()).await,
            context: duplicate,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, JavaScriptHostError::Contract(_)));
    supervisor.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn first_capability_demand_starts_and_loads_only_its_mount() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let mount = context(temp.path(), "mount-demand", 'c');
    tokio::fs::create_dir_all(&mount.data_dir).await.unwrap();

    let value = supervisor
        .invoke_demand(
            MountLoadDemand {
                context: mount.clone(),
                module: module(mount.target.clone()).await,
            },
            contribution(mount.target),
            ActionId::from("echo"),
            StrictJsonValue(json!({"lazy": true})),
        )
        .await
        .unwrap();

    assert_eq!(value.0["input"]["lazy"], true);
    let JavaScriptHostState::Running { generation, .. } = supervisor.state() else {
        panic!("first capability demand must start one Host generation");
    };
    assert_eq!(generation, 1);
    assert!(matches!(
        supervisor
            .start_invocation(
                contribution(target("never-loaded", 'd')),
                ActionId::from("echo"),
                StrictJsonValue(json!({})),
            )
            .await,
        Err(JavaScriptHostError::MountNotResident(_))
    ));
    supervisor.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn ordinary_rejection_isolated_to_one_invocation() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&supervisor, &temp, "mount-a", 'a').await;

    let error = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("reject"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, JavaScriptHostError::RequestFailed { .. }));
    assert_eq!(supervisor.process_count(), 1);

    let value = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({"still": "running"})),
        )
        .await
        .unwrap();
    assert_eq!(value.0["input"]["still"], "running");
    supervisor.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn context_and_resource_requests_use_the_exact_resident_mount() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&supervisor, &temp, "mount-a", 'a').await;
    let contribution = contribution(target("mount-a", 'a'));

    let context = supervisor
        .contribute_context(
            contribution.clone(),
            CanonicalSchemaRef::from("schema://fixture/context"),
        )
        .await
        .unwrap();
    assert_eq!(context.0["schema_ref"], "schema://fixture/context");
    assert_eq!(context.0["mount_id"], "mount-a");

    let resource = supervisor
        .acquire_resource(
            contribution.clone(),
            ResourceBindingId::from("binding-a"),
            ResourceKind::from("fixture.resource"),
            StrictJsonValue(json!({"path": "fixture"})),
        )
        .await
        .unwrap();
    assert_eq!(resource.host_generation, generation);
    assert_eq!(resource.handle_id, "mount-a:binding-a");
    supervisor.release_resource(&resource).await.unwrap();

    let release_count = supervisor
        .invoke(
            contribution,
            ActionId::from("resource_release_count"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(release_count.0["count"], 1);
    supervisor.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn stale_resource_release_does_not_start_or_touch_a_new_generation() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let first_generation = load(&supervisor, &temp, "mount-a", 'a').await;
    let resource = supervisor
        .acquire_resource(
            contribution(target("mount-a", 'a')),
            ResourceBindingId::from("binding-a"),
            ResourceKind::from("fixture.resource"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    supervisor.stop_generation(first_generation).await.unwrap();

    let second_generation = load(&supervisor, &temp, "mount-b", 'b').await;
    assert_ne!(first_generation, second_generation);
    supervisor.release_resource(&resource).await.unwrap();
    assert_eq!(
        supervisor.state(),
        JavaScriptHostState::Running {
            generation: second_generation,
            process_id: match supervisor.state() {
                JavaScriptHostState::Running { process_id, .. } => process_id,
                _ => unreachable!(),
            },
        }
    );
    supervisor.stop_generation(second_generation).await.unwrap();
}

#[tokio::test]
async fn cancellation_is_request_scoped_and_generation_stays_running() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&supervisor, &temp, "mount-a", 'a').await;
    let pending = supervisor
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("wait_for_cancel"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    let request_id = pending.request_id.clone();
    supervisor.cancel(request_id).await.unwrap();
    let error = pending.wait().await.unwrap_err();
    assert!(matches!(error, JavaScriptHostError::RequestFailed { .. }));
    assert_eq!(supervisor.process_count(), 1);
    supervisor.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn crash_fails_generation_and_next_demand_starts_without_replay() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    load(&supervisor, &temp, "mount-a", 'a').await;

    let error = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("crash"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, JavaScriptHostError::HostFailure { generation: 1, .. }));
    wait_until_stopped(&supervisor).await;
    assert_eq!(supervisor.process_count(), 0);

    let generation = load(&supervisor, &temp, "mount-b", 'b').await;
    assert_eq!(generation, 2);
    let old_mount = supervisor
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        old_mount,
        JavaScriptHostError::MountNotResident(_)
    ));
    let value = supervisor
        .invoke(
            contribution(target("mount-b", 'b')),
            ActionId::from("echo"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(value.0["mount_id"], "mount-b");
    supervisor.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn watchdog_fails_all_pending_requests_and_reaps_generation() {
    let supervisor = supervisor(Duration::from_millis(150)).await;
    let temp = TempDir::new().unwrap();
    load(&supervisor, &temp, "mount-a", 'a').await;

    let first = supervisor
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("hang"),
            StrictJsonValue(json!({"request": 1})),
        )
        .await
        .unwrap();
    let second = supervisor
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("hang"),
            StrictJsonValue(json!({"request": 2})),
        )
        .await
        .unwrap();
    let (first, second) = tokio::join!(first.wait(), second.wait());
    assert!(matches!(
        first.unwrap_err(),
        JavaScriptHostError::HostFailure { generation: 1, .. }
    ));
    assert!(matches!(
        second.unwrap_err(),
        JavaScriptHostError::HostFailure { generation: 1, .. }
    ));
    wait_until_stopped(&supervisor).await;
    assert_eq!(supervisor.process_count(), 0);
}

#[tokio::test]
async fn process_tree_cleanup_stops_spawned_child_heartbeat() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    load(&supervisor, &temp, "mount-a", 'a').await;
    let heartbeat = temp.path().join("heartbeat.txt");

    let error = supervisor
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("child_then_crash"),
            StrictJsonValue(json!({"heartbeat": heartbeat})),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, JavaScriptHostError::HostFailure { .. }));
    wait_until_stopped(&supervisor).await;

    tokio::time::sleep(Duration::from_millis(150)).await;
    let first = tokio::fs::metadata(&heartbeat).await.unwrap().len();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = tokio::fs::metadata(&heartbeat).await.unwrap().len();
    assert_eq!(first, second, "child heartbeat continued after cleanup");
}

#[tokio::test]
async fn late_generation_response_is_rejected_and_host_is_reaped() {
    let host = fixture("late-host.mjs")
        .canonicalize()
        .expect("late Host fixture exists");
    let supervisor =
        supervisor_with_host(Duration::from_secs(1), host).await;
    let temp = TempDir::new().unwrap();
    let context = context(temp.path(), "mount-a", 'a');
    tokio::fs::create_dir_all(&context.data_dir).await.unwrap();

    let error = supervisor
        .load_mount(MountLoadDemand {
            module: module(context.target.clone()).await,
            context,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        JavaScriptHostError::HostFailure { generation: 1, .. }
    ));
    wait_until_stopped(&supervisor).await;
    assert_eq!(supervisor.process_count(), 0);
}

#[tokio::test]
async fn resident_mount_requires_stop_generation_for_commit_fence() {
    let supervisor = supervisor(Duration::from_secs(2)).await;
    let temp = TempDir::new().unwrap();
    let generation = load(&supervisor, &temp, "mount-a", 'a').await;
    let mount_id = PluginMountId::from("mount-a");

    assert!(matches!(
        supervisor.commit_fence_for_mount(&mount_id).await,
        Err(JavaScriptHostError::NotQuiescent { generation: 1 })
    ));
    let pending = supervisor
        .start_invocation(
            contribution(target("mount-a", 'a')),
            ActionId::from("wait_for_cancel"),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert!(matches!(
        supervisor.stop_generation(generation).await,
        Err(JavaScriptHostError::NotQuiescent { generation: 1 })
    ));
    supervisor.cancel(pending.request_id.clone()).await.unwrap();
    assert!(matches!(
        pending.wait().await,
        Err(JavaScriptHostError::RequestFailed { .. })
    ));
    let fence = supervisor.stop_generation(generation).await.unwrap();
    assert!(matches!(
        fence,
        PluginHostCommitFence::ResidentFenced {
            host_generation: 1,
            ..
        }
    ));
    assert_eq!(
        supervisor.commit_fence_for_mount(&mount_id).await.unwrap(),
        PluginHostCommitFence::NotResident
    );
}
