//! Drop/timeout must propagate through the existing Service owner, without
//! relying on a caller continuing to poll its invocation future.
use super::*;
use nomifun_plugin_platform::runtime::{
    InMemoryPluginRuntimeServiceHost, PluginRuntimeServiceHostPort,
};

const TRACKED_SERVICE: &str = r#"
export async function start() {
  let started = 0;
  let aborted = 0;
  return {
    async invoke({method, signal}) {
      if (method === 'stats') return {started, aborted};
      started++;
      if (method === 'ignore') return new Promise(() => {});
      return new Promise((resolve, reject) => {
        signal.addEventListener('abort', () => {
          aborted++;
          reject(new Error('cancelled'));
        }, {once: true});
      });
    },
    async dispose() {},
  };
}
"#;

async fn process_counts(
    process: &Arc<dyn nomifun_plugin_platform::runtime::PluginRuntimeServiceProcess>,
    spec: &ResolvedPluginServiceSpec,
    started: u64,
    aborted: u64,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let result = invoke(
            process,
            spec,
            1,
            "stats",
            "stats",
            json!({}),
            PluginRuntimeCallCancellation::default(),
        )
        .await
        .unwrap();
        if result.0 == json!({"started":started,"aborted":aborted}) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "unexpected counters: {}",
            result.0
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn abandoned_process_call_is_cancelled_without_affecting_another_call() {
    let Some(node) = node_executable() else {
        eprintln!("Node unavailable; skipping real Service cancellation regression");
        return;
    };
    let directory = TempDir::new().unwrap();
    let module = write_module(&directory, "main.mjs", TRACKED_SERVICE);
    let spec = service_spec(
        &node,
        TRACKED_SERVICE.as_bytes(),
        "drop-process",
        1,
        PluginServiceLifecycle::OnDemand,
    );
    let process = factory(&node, &module, Duration::from_secs(5))
        .start(PluginRuntimeServiceLaunch {
            spec: spec.clone(),
            host_generation: 1,
        })
        .await
        .unwrap();
    let mut calls = Vec::new();
    let cancellation = PluginRuntimeCallCancellation::default();
    for id in ["abandoned", "retained"] {
        let (process, spec, token) = (process.clone(), spec.clone(), cancellation.clone());
        // The abandoned invocation does not cancel a shared caller token;
        // this test uses distinct tokens, as production calls do.
        let token = if id == "abandoned" {
            PluginRuntimeCallCancellation::default()
        } else {
            token
        };
        calls.push(tokio::spawn(async move {
            invoke(&process, &spec, 1, id, "wait", json!({}), token).await
        }));
    }
    process_counts(&process, &spec, 2, 0).await;
    let abandoned = calls.remove(0);
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    process_counts(&process, &spec, 2, 1).await;
    assert!(
        !calls[0].is_finished(),
        "an unrelated request was cancelled"
    );
    cancellation.cancel();
    assert!(calls.remove(0).await.unwrap().is_err());
    process_counts(&process, &spec, 2, 2).await;
    assert!(process.terminal_result().is_none());
    process.stop().await;
}

async fn host_stats(
    host: &InMemoryPluginRuntimeServiceHost,
    spec: &ResolvedPluginServiceSpec,
) -> serde_json::Value {
    let successful = PluginRuntimeCallCancellation::default();
    let result = host
        .invoke(
            spec,
            "stats".into(),
            "stats".into(),
            StrictJsonValue(json!({})),
            successful.clone(),
            1,
        )
        .await
        .unwrap();
    assert!(
        !successful.is_canceled(),
        "normal completion must not cancel its token"
    );
    result.0
}

async fn host_counts(
    host: &InMemoryPluginRuntimeServiceHost,
    spec: &ResolvedPluginServiceSpec,
    started: u64,
    aborted: u64,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let result = host_stats(host, spec).await;
        if result == json!({"started":started,"aborted":aborted}) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "unexpected counters: {result}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn abandoned_host_call_releases_registration_and_does_not_prevent_idle_reaping() {
    let Some(node) = node_executable() else {
        eprintln!("Node unavailable; skipping real Service cancellation regression");
        return;
    };
    let directory = TempDir::new().unwrap();
    let module = write_module(&directory, "main.mjs", TRACKED_SERVICE);
    let spec = service_spec(
        &node,
        TRACKED_SERVICE.as_bytes(),
        "drop-host",
        1,
        PluginServiceLifecycle::OnDemand,
    );
    let host = Arc::new(InMemoryPluginRuntimeServiceHost::new(Arc::new(factory(
        &node,
        &module,
        Duration::from_secs(5),
    ))));
    host.bind_active(spec.clone(), true).await.unwrap();
    // Establish the on-demand process before measuring request admission.
    // The two-second counter deadline is not a budget for hashing the Node
    // executable and performing the initial Hello handshake on a cold host.
    assert_eq!(host_stats(&host, &spec).await, json!({"started": 0, "aborted": 0}));
    let mut calls = Vec::new();
    let mut tokens = Vec::new();
    for id in ["abandoned", "retained"] {
        let (host, spec) = (host.clone(), spec.clone());
        let token = PluginRuntimeCallCancellation::default();
        tokens.push(token.clone());
        calls.push(tokio::spawn(async move {
            host.invoke(
                &spec,
                id.into(),
                "wait".into(),
                StrictJsonValue(json!({})),
                token,
                1,
            )
            .await
        }));
    }
    host_counts(&host, &spec, 2, 0).await;
    let abandoned = calls.remove(0);
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    assert!(tokens[0].is_canceled());
    host_counts(&host, &spec, 2, 1).await;
    assert!(!tokens[1].is_canceled());
    assert!(
        host.reap_idle(100_000, 1).await.unwrap().is_empty(),
        "live call must retain the host"
    );
    // Once Node confirms cancellation, the abandoned call ID is reusable;
    // a stale Host registration must not reject it as a duplicate forever.
    host.invoke(
        &spec,
        "abandoned".into(),
        "stats".into(),
        StrictJsonValue(json!({})),
        PluginRuntimeCallCancellation::default(),
        1,
    )
    .await
    .unwrap();
    let retained = calls.remove(0);
    retained.abort();
    assert!(retained.await.unwrap_err().is_cancelled());
    assert!(tokens[1].is_canceled());
    // No subsequent invocation may be necessary to prune the final dropped
    // registration: maintenance alone must be able to reap this host.
    assert_eq!(
        host.reap_idle(100_000, 1).await.unwrap(),
        vec![spec.plugin_product_id.clone()]
    );
    assert!(matches!(
        host.state(&spec.plugin_product_id).await,
        Some(nomifun_plugin_platform::runtime::PluginRuntimeServiceHostState::Stopped)
    ));
}

#[tokio::test]
async fn abandoned_uncooperative_call_still_expires_at_the_watchdog() {
    let Some(node) = node_executable() else {
        eprintln!("Node unavailable; skipping real Service cancellation regression");
        return;
    };
    let directory = TempDir::new().unwrap();
    let module = write_module(&directory, "main.mjs", TRACKED_SERVICE);
    let spec = service_spec(
        &node,
        TRACKED_SERVICE.as_bytes(),
        "drop-watchdog",
        1,
        PluginServiceLifecycle::OnDemand,
    );
    let process = factory(&node, &module, Duration::from_secs(1))
        .start(PluginRuntimeServiceLaunch {
            spec: spec.clone(),
            host_generation: 1,
        })
        .await
        .unwrap();
    let pending = {
        let (process, spec) = (process.clone(), spec.clone());
        tokio::spawn(async move {
            invoke(
                &process,
                &spec,
                1,
                "ignored",
                "ignore",
                json!({}),
                PluginRuntimeCallCancellation::default(),
            )
            .await
        })
    };
    process_counts(&process, &spec, 1, 0).await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(result) = process.terminal_result() {
            assert!(result.unwrap_err().contains("watchdog timed out"));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cancel ACK must not keep the ignored call alive"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    process.stop().await;
}
