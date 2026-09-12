use super::*;

async fn config() -> JavaScriptHostConfig {
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
    let mut config = JavaScriptHostConfig::for_host_module(
        executable,
        probe.fingerprint.unwrap(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/extension-host.mjs"),
    );
    config.limits.shutdown_timeout = Duration::from_millis(30);
    config
}

async fn assert_unproven_cleanup_blocks_admission(failed_handle: bool) {
    let config = config().await;
    let mut admitted = Vec::new();
    for operation in ["restart", "mount_fence", "stop", "confirm_stopped"] {
        let host = ExtensionHostSupervisor::new(config.clone()).unwrap();
        let mut builder = ChildProcessBuilder::new(&config.node_executable);
        builder
            .args(["-e", "setTimeout(() => {}, 10000)"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = builder.spawn_managed().unwrap();
        {
            let mut state = host.state.lock().await;
            state.next_generation = 1;
            state.cleanup = process.cleanup_receipt();
            if failed_handle {
                let (commands, _receiver) = mpsc::channel(1);
                let (_, failed) = watch::channel(JavaScriptHostState::Failed {
                    generation: 1,
                    reason: "cleanup is unresolved".into(),
                });
                state.current = Some(GenerationHandle {
                    host_kind: host.host_kind,
                    generation: 1,
                    commands,
                    state: failed,
                    mounts: Arc::new(RwLock::new(BTreeMap::new())),
                    admission: Arc::new(RwLock::new(())),
                });
            }
        }
        let result = match operation {
            "restart" => host.ensure_generation().await.map(|_| ()),
            "mount_fence" => host
                .commit_fence_for_mount(&PluginMountId::from("mount-a"))
                .await
                .map(|_| ()),
            "stop" => host.stop_generation(1).await.map(|_| ()),
            _ => host.confirm_stopped().await,
        };
        if result.is_ok() {
            admitted.push(operation);
        }
        if result.is_err() {
            let state = host.state.lock().await;
            assert!(state.cleanup.is_some(), "failed wait discarded the receipt");
            assert_eq!(
                state.next_generation, 1,
                "blocked admission consumed a generation"
            );
        }
        // Always clean up, including when the old implementation wrongly starts
        // a new generation. The real outstanding receipt is settled first.
        process.shutdown().await.unwrap();
        if let JavaScriptHostState::Running { generation, .. } = host.state() {
            let _ = host.stop_generation(generation).await;
        }
        host.confirm_stopped().await.unwrap();
        assert!(
            host.state.lock().await.cleanup.is_none(),
            "completed proof was not retired"
        );
    }
    assert!(
        admitted.is_empty(),
        "unproven cleanup admitted {admitted:?}"
    );
}

#[tokio::test]
async fn cancelled_startup_cleanup_blocks_restart_and_not_resident_fences() {
    assert_unproven_cleanup_blocks_admission(false).await;
}

#[tokio::test]
async fn failed_generation_cleanup_blocks_restart_and_not_resident_fences() {
    assert_unproven_cleanup_blocks_admission(true).await;
}

#[tokio::test]
async fn cleanup_confirmation_does_not_stop_a_running_generation() {
    let mut config = config().await;
    config.limits.shutdown_timeout = Duration::from_secs(2);
    let host = ExtensionHostSupervisor::new(config).unwrap();
    let generation = host.ensure_generation().await.unwrap().generation;
    let confirmation = host.confirm_stopped().await;
    let still_running = matches!(host.state(), JavaScriptHostState::Running { .. });
    host.stop_generation(generation).await.unwrap();
    host.confirm_stopped().await.unwrap();
    assert!(matches!(
        confirmation,
        Err(JavaScriptHostError::NotQuiescent { .. })
    ));
    assert!(still_running);
}
