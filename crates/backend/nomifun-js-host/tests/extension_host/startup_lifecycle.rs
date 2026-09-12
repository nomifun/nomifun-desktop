use super::*;
use nomi_process_runtime::ChildProcessBuilder;
use std::sync::Arc;

async fn setup(mode: &str) -> (ExtensionHostSupervisor, TempDir) {
    let temp = TempDir::new().unwrap();
    let host_path = temp.path().join("startup-host.mjs");
    tokio::fs::copy(fixture("startup-host.mjs"), &host_path)
        .await
        .unwrap();
    tokio::fs::write(
        temp.path().join("mode.json"),
        json!({"mode": mode}).to_string(),
    )
    .await
    .unwrap();
    let mut config = host_config(Duration::from_secs(2), host_path).await;
    // Cancellation is triggered by the fixture's readiness file, not the
    // Hello watchdog. Parallel runtime probes may delay process creation.
    config.limits.hello_timeout = if mode == "cancel" {
        Duration::from_secs(10)
    } else {
        Duration::from_millis(500)
    };
    config.limits.shutdown_timeout = Duration::from_secs(1);
    (ExtensionHostSupervisor::new(config).unwrap(), temp)
}

async fn demand(
    host: &ExtensionHostSupervisor,
    temp: &TempDir,
) -> Result<u64, JavaScriptHostError> {
    let mount = context(temp.path(), "mount-a", 'a');
    host.load_mount(MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stderr_is_drained_before_waiting_for_hello() {
    let (host, temp) = setup("noisy").await;
    let result = demand(&host, &temp).await;
    if let Ok(generation) = result {
        host.stop_generation(generation).await.unwrap();
    }
    assert!(result.is_ok(), "startup stderr blocked Hello: {result:?}");
}

async fn assert_safe_startup_error(mode: &str) {
    let (host, temp) = setup(mode).await;
    let error = demand(&host, &temp).await.unwrap_err().to_string();
    assert!(
        !error.contains("fixture-startup-secret"),
        "stderr leaked: {error}"
    );
    assert!(
        !error.contains("fixture-hello-secret"),
        "Hello content leaked: {error}"
    );
}

#[tokio::test]
async fn startup_errors_do_not_expose_stderr() {
    assert_safe_startup_error("secret").await;
}

#[tokio::test]
async fn startup_errors_do_not_expose_untrusted_hello_values() {
    assert_safe_startup_error("invalid").await;
}

#[tokio::test]
async fn startup_failure_publishes_its_generation_and_allows_retry() {
    let (host, temp) = setup("timeout").await;
    let error = demand(&host, &temp).await.unwrap_err();
    assert!(matches!(error, JavaScriptHostError::HelloRejected(_)));
    let failed = host.subscribe_state().borrow().clone();
    tokio::fs::write(
        temp.path().join("mode.json"),
        json!({"mode": "valid"}).to_string(),
    )
    .await
    .unwrap();
    let generation = demand(&host, &temp).await.unwrap();
    host.stop_generation(generation).await.unwrap();
    assert!(
        matches!(failed, JavaScriptHostState::Failed { generation: 1, .. }),
        "startup failure remained invisible: {failed:?}"
    );
    assert_eq!(generation, 2);
}

#[tokio::test]
async fn hello_must_bind_the_exact_process_runtime_role_and_generation() {
    for mode in [
        "wrong-generation",
        "wrong-process",
        "wrong-role",
        "wrong-runtime",
        "wrong-contract",
    ] {
        let (host, temp) = setup(mode).await;
        let result = demand(&host, &temp).await;
        if let Ok(generation) = result {
            host.stop_generation(generation).await.unwrap();
        }
        assert!(
            matches!(result, Err(JavaScriptHostError::HelloRejected(_))),
            "accepted {mode}: {result:?}"
        );
        assert!(matches!(
            host.subscribe_state().borrow().clone(),
            JavaScriptHostState::Failed { generation: 1, .. }
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_before_hello_reaps_the_root_and_descendant() {
    let (host, temp) = setup("cancel").await;
    let host = Arc::new(host);
    let mount = context(temp.path(), "mount-a", 'a');
    let mount_demand = MountLoadDemand {
        module: module(mount.target.clone()).await,
        context: mount,
    };
    let task = tokio::spawn({
        let host = host.clone();
        async move { host.load_mount(mount_demand).await }
    });
    let pids = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(bytes) = tokio::fs::read(temp.path().join("started.json")).await {
                if let Ok(pids) = serde_json::from_slice::<Vec<u32>>(&bytes) {
                    break pids;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let script = format!(
        "const pids={pids:?}; let alive=false; for(const pid of pids){{try{{process.kill(pid,0);alive=true}}catch(e){{if(e.code!=='ESRCH')throw e}}}}process.exit(alive?1:0)"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let mut probe = ChildProcessBuilder::new(which_node());
            probe.args(["-e", script.as_str()]);
            let output = probe.output().await.unwrap();
            if output.status.success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancelled startup must reap its exact process tree before fallback exit");
    assert_eq!(host.process_count(), 0);
    tokio::fs::write(
        temp.path().join("mode.json"),
        json!({"mode": "valid"}).to_string(),
    )
    .await
    .unwrap();
    let generation = demand(&host, &temp).await.unwrap();
    assert_eq!(generation, 2, "a proven cancelled startup must allow retry");
    host.stop_generation(generation).await.unwrap();
}
