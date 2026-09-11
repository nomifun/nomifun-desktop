use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use nomi_process_runtime::probe_process_identity;
use nomifun_agent_contracts::{
    ArtifactId, DigestHex, MiniAppBridgeCallId, MiniAppId, MiniAppKvHandleDescriptor,
    MiniAppKvHandleId, MiniAppReleaseId, MiniAppReleaseRef, MiniAppServiceLifecycle,
    MiniAppServiceRuntimeFingerprint, MiniAppServiceStorageDescriptor,
    ResolvedMiniAppServiceSpec, ResolvedMiniAppServiceSpecInputs, RuntimeInstallationId,
    RuntimeTarget, StrictJsonValue, VersionString, digest_bytes,
};
use nomifun_miniapp_platform::{
    FixedMiniAppServiceModuleResolver, MiniAppCallCancellation,
    MiniAppServiceGenerationFence, MiniAppServiceInvocation, MiniAppServiceLaunch,
    MiniAppServiceProcessError, MiniAppServiceProcessFactory, MiniAppServiceProcessLimits,
    NodeMiniAppServiceProcessFactory,
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

const SERVICE_MODULE: &str = r#"
import { spawn } from "node:child_process";

export async function start(context) {
  return {
    async invoke({ method, payload, signal }) {
      if (method === "echo") {
        return {
          method,
          payload,
          hostGeneration: context.hostGeneration,
          miniappId: context.miniappId,
        };
      }
      if (method === "hang") {
        return await new Promise((resolve, reject) => {
          signal.addEventListener(
            "abort",
            () => {
              const error = new Error("canceled");
              error.name = "AbortError";
              reject(error);
            },
            { once: true },
          );
        });
      }
      if (method === "hang_forever") {
        return await new Promise(() => {});
      }
      if (method === "crash") {
        process.exit(86);
        return await new Promise(() => {});
      }
      if (method === "spawn_child") {
        const marker = payload.marker;
        const childSource =
          "const fs=require('node:fs');const p=process.argv[1];" +
          "setInterval(()=>fs.appendFileSync(p,'x'),20);";
        const child = spawn(process.execPath, ["-e", childSource, marker], {
          stdio: "ignore",
        });
        child.unref();
        return { pid: child.pid };
      }
      throw new Error(`unknown method: ${method}`);
    },
    async dispose() {},
  };
}
"#;

const INVALID_SERVICE_MODULE: &str = "export const value = 1;\n";

fn node_executable() -> Option<PathBuf> {
    let discovered = which::which("node").ok()?;
    std::fs::canonicalize(discovered).ok()
}

fn runtime_fingerprint(node: &Path) -> MiniAppServiceRuntimeFingerprint {
    let bytes = std::fs::read(node).expect("read Node executable");
    let output = Command::new(node)
        .arg("--version")
        .output()
        .expect("query Node version");
    assert!(output.status.success(), "Node --version failed");
    let version = String::from_utf8(output.stdout)
        .expect("Node version is UTF-8")
        .trim()
        .trim_start_matches('v')
        .to_owned();
    MiniAppServiceRuntimeFingerprint {
        runtime_installation_id: RuntimeInstallationId::from(format!(
            "test-node-{}",
            &digest_bytes(&bytes).as_ref()[..16]
        )),
        runtime_target: RuntimeTarget::from(format!(
            "{}-{}-test",
            std::env::consts::ARCH,
            std::env::consts::OS
        )),
        runtime_executable_digest: digest_bytes(&bytes),
        node_version: VersionString::from(version),
    }
}

fn digest(seed: &str) -> DigestHex {
    digest_bytes(seed.as_bytes())
}

fn service_spec(
    node: &Path,
    module_bytes: &[u8],
    miniapp_id: &str,
    epoch: u64,
    lifecycle: MiniAppServiceLifecycle,
) -> ResolvedMiniAppServiceSpec {
    let miniapp_id = MiniAppId::from(miniapp_id);
    ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
        miniapp_id: miniapp_id.clone(),
        release: MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from(Uuid::now_v7().to_string()),
            artifact_id: ArtifactId::from(Uuid::now_v7().to_string()),
            release_digest: digest("release"),
            manifest_digest: digest("manifest"),
        },
        active_release_epoch: epoch,
        service_module_digest: digest_bytes(module_bytes),
        lifecycle,
        host_protocol_version:
            nomifun_agent_contracts::MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
        sdk_contract_version:
            nomifun_agent_contracts::MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
        runtime: runtime_fingerprint(node),
        config_schema_digest: digest("config-schema"),
        config_snapshot_digest: digest("config-snapshot"),
        credential_slots_digest: digest("credential-slots"),
        resource_contract_digest: digest("resource-contract"),
        resource_bindings_digest: digest("resource-bindings"),
        runtime_requirements_digest: digest("runtime-requirements"),
        bridge_contract_digest: digest("bridge-contract"),
        contribution_set_digest: digest("contribution-set"),
        storage: MiniAppServiceStorageDescriptor {
            kv: MiniAppKvHandleDescriptor {
                handle_id: MiniAppKvHandleId::from(format!(
                    "test-kv-{}",
                    miniapp_id.as_ref()
                )),
                miniapp_id,
                namespace_revision: 1,
            },
            files_dir: None,
            private_database: None,
        },
    })
    .expect("valid resolved Service spec")
}

fn fence(spec: &ResolvedMiniAppServiceSpec, generation: u64) -> MiniAppServiceGenerationFence {
    MiniAppServiceGenerationFence {
        miniapp_id: spec.miniapp_id.clone(),
        release: spec.release.clone(),
        active_release_epoch: spec.active_release_epoch,
        service_run_key: spec.service_run_key.clone(),
        host_generation: generation,
    }
}

fn factory(
    node: &Path,
    module: &Path,
    request_timeout: Duration,
) -> NodeMiniAppServiceProcessFactory {
    NodeMiniAppServiceProcessFactory::new(
        node.to_path_buf(),
        Arc::new(FixedMiniAppServiceModuleResolver::new(module)),
    )
    .expect("construct Node Service factory")
    .with_limits(MiniAppServiceProcessLimits {
        hello_timeout: Duration::from_secs(5),
        request_timeout,
        shutdown_timeout: Duration::from_secs(3),
        max_frame_bytes: 1024 * 1024,
        command_queue_capacity: 32,
        cancellation_poll_interval: Duration::from_millis(5),
    })
    .expect("valid test limits")
}

fn write_module(directory: &TempDir, name: &str, source: &str) -> PathBuf {
    let path = directory.path().join(name);
    std::fs::write(&path, source).expect("write Service module");
    path
}

async fn invoke(
    process: &Arc<dyn nomifun_miniapp_platform::MiniAppServiceProcess>,
    spec: &ResolvedMiniAppServiceSpec,
    generation: u64,
    call_id: &str,
    method: &str,
    payload: serde_json::Value,
    cancellation: MiniAppCallCancellation,
) -> Result<StrictJsonValue, MiniAppServiceProcessError> {
    process
        .invoke(
            MiniAppServiceInvocation {
                fence: fence(spec, generation),
                call_id: MiniAppBridgeCallId::from(call_id),
                method: method.to_owned(),
                payload: StrictJsonValue(payload),
            },
            cancellation,
        )
        .await
}

async fn wait_for_file_growth(path: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut previous = 0;
    loop {
        if let Ok(metadata) = tokio::fs::metadata(path).await
            && metadata.len() > previous
        {
            if previous > 0 {
                return;
            }
            previous = metadata.len();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "child heartbeat did not start"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_process_gone(pid: u32) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if probe_process_identity(pid)
            .expect("probe child process identity")
            .is_none()
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "child process {pid} survived Service Host cleanup"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn real_node_service_invokes_cancels_and_rejects_stale_generation() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping MiniApp Service process test");
        return;
    };
    let directory = TempDir::new().expect("temporary Service directory");
    let module = write_module(&directory, "main.mjs", SERVICE_MODULE);
    let spec = service_spec(
        &node,
        SERVICE_MODULE.as_bytes(),
        "miniapp-service-a",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );
    let process = factory(&node, &module, Duration::from_secs(2))
        .start(MiniAppServiceLaunch {
            spec: spec.clone(),
            host_generation: 1,
        })
        .await
        .expect("start Service process");

    let echoed = invoke(
        &process,
        &spec,
        1,
        "call-echo",
        "echo",
        json!({"value": 42}),
        MiniAppCallCancellation::default(),
    )
    .await
    .expect("invoke echo");
    assert_eq!(echoed.0["payload"], json!({"value": 42}));
    assert_eq!(echoed.0["hostGeneration"], 1);
    assert_eq!(echoed.0["miniappId"], "miniapp-service-a");

    let stale = invoke(
        &process,
        &spec,
        2,
        "call-stale",
        "echo",
        json!({}),
        MiniAppCallCancellation::default(),
    )
    .await
    .expect_err("stale generation must be rejected");
    assert!(matches!(stale, MiniAppServiceProcessError::Rejected(_)));

    let cancellation = MiniAppCallCancellation::default();
    let pending = invoke(
        &process,
        &spec,
        1,
        "call-cancel",
        "hang",
        json!({}),
        cancellation.clone(),
    );
    tokio::pin!(pending);
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancellation.cancel();
    let canceled = pending
        .await
        .expect_err("canceled invocation must not produce a value");
    assert!(matches!(canceled, MiniAppServiceProcessError::Rejected(_)));

    process.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn node_path_alias_is_resolved_once_and_still_requires_the_selected_digest() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping Unix Service path-alias regression");
        return;
    };
    let directory = TempDir::new().unwrap();
    let alias = directory.path().join("node");
    std::os::unix::fs::symlink(&node, &alias).unwrap();
    let module = write_module(&directory, "main.mjs", SERVICE_MODULE);
    let factory = factory(&alias, &module, Duration::from_secs(2));
    let spec = service_spec(
        &node,
        SERVICE_MODULE.as_bytes(),
        "miniapp-node-alias",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );

    // A package manager can retarget the PATH alias after admission. The
    // factory must continue using the originally resolved executable.
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(directory.path().join("missing-node"), &alias).unwrap();
    let process = factory.start(MiniAppServiceLaunch {
        spec: spec.clone(),
        host_generation: 1,
    }).await.expect("launch pinned executable, not retargeted PATH alias");
    let result = invoke(
        &process, &spec, 1, "alias-echo", "echo", json!({"unix": true}),
        MiniAppCallCancellation::default(),
    ).await.unwrap();
    assert_eq!(result.0["payload"], json!({"unix": true}));
    process.stop().await;

    let mut wrong = spec;
    wrong.runtime.runtime_executable_digest = digest_bytes(b"different executable");
    let error = factory.start(MiniAppServiceLaunch {
        spec: wrong,
        host_generation: 2,
    }).await.err().expect("wrong selected digest must be rejected");
    assert!(error.to_string().contains("digest mismatch"), "{error}");
}

#[cfg(windows)]
#[tokio::test]
async fn real_node_service_starts_from_an_extended_length_module_path() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping long-path MiniApp Service process test");
        return;
    };
    let directory = TempDir::new().expect("temporary Service directory");
    let mut module_root = directory.path().to_path_buf();
    for index in 0..10 {
        module_root.push(format!("managed-release-segment-{index:02}"));
    }
    std::fs::create_dir_all(&module_root).expect("create extended-length module root");
    let module = module_root.join("main.mjs");
    std::fs::write(&module, SERVICE_MODULE).expect("write extended-length Service module");
    assert!(module.as_os_str().len() > 300);
    let spec = service_spec(
        &node,
        SERVICE_MODULE.as_bytes(),
        "miniapp-service-long-path",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );
    let process = factory(&node, &module, Duration::from_secs(2))
        .start(MiniAppServiceLaunch {
            spec,
            host_generation: 1,
        })
        .await
        .expect("start Service process from an extended-length module path");
    process.stop().await;
}

#[tokio::test]
async fn crash_is_isolated_and_stop_reaps_spawned_process_tree() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping MiniApp Service process test");
        return;
    };
    let directory = TempDir::new().expect("temporary Service directory");
    let module = write_module(&directory, "main.mjs", SERVICE_MODULE);
    let factory = factory(&node, &module, Duration::from_secs(2));
    let first_spec = service_spec(
        &node,
        SERVICE_MODULE.as_bytes(),
        "miniapp-service-first",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );
    let second_spec = service_spec(
        &node,
        SERVICE_MODULE.as_bytes(),
        "miniapp-service-second",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );
    let first = factory
        .start(MiniAppServiceLaunch {
            spec: first_spec.clone(),
            host_generation: 1,
        })
        .await
        .expect("start first Service process");
    let second = factory
        .start(MiniAppServiceLaunch {
            spec: second_spec.clone(),
            host_generation: 1,
        })
        .await
        .expect("start second Service process");

    let crash = invoke(
        &first,
        &first_spec,
        1,
        "call-crash",
        "crash",
        json!({}),
        MiniAppCallCancellation::default(),
    )
    .await
    .expect_err("first Service must crash");
    assert!(matches!(crash, MiniAppServiceProcessError::Crashed(_)));

    let healthy = invoke(
        &second,
        &second_spec,
        1,
        "call-after-crash",
        "echo",
        json!({"healthy": true}),
        MiniAppCallCancellation::default(),
    )
    .await
    .expect("second Service remains healthy");
    assert_eq!(healthy.0["payload"]["healthy"], true);

    let marker = directory.path().join("child-heartbeat.txt");
    let spawned = invoke(
        &second,
        &second_spec,
        1,
        "call-spawn",
        "spawn_child",
        json!({"marker": marker}),
        MiniAppCallCancellation::default(),
    )
    .await
    .expect("spawn child process");
    let child_pid = spawned.0["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("Service returned a child PID");
    wait_for_file_growth(&marker).await;
    assert!(
        probe_process_identity(child_pid)
            .expect("probe live child")
            .is_some()
    );

    second.stop().await;
    wait_for_process_gone(child_pid).await;
    let size_after_stop = tokio::fs::metadata(&marker)
        .await
        .expect("heartbeat marker remains readable")
        .len();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        tokio::fs::metadata(&marker)
            .await
            .expect("heartbeat marker remains readable")
            .len(),
        size_after_stop,
        "child heartbeat continued after Service Host cleanup"
    );
}

#[tokio::test]
async fn invalid_module_and_request_timeout_fail_closed() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping MiniApp Service process test");
        return;
    };
    let directory = TempDir::new().expect("temporary Service directory");
    let invalid_module = write_module(&directory, "invalid-main.mjs", INVALID_SERVICE_MODULE);
    let invalid_spec = service_spec(
        &node,
        INVALID_SERVICE_MODULE.as_bytes(),
        "miniapp-service-invalid",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );
    let error = factory(&node, &invalid_module, Duration::from_millis(150))
        .start(MiniAppServiceLaunch {
            spec: invalid_spec,
            host_generation: 1,
        })
        .await
        .err()
        .expect("module without start(context) must fail");
    assert!(error.to_string().contains("Hello"));

    let module = write_module(&directory, "main.mjs", SERVICE_MODULE);
    let spec = service_spec(
        &node,
        SERVICE_MODULE.as_bytes(),
        "miniapp-service-timeout",
        1,
        MiniAppServiceLifecycle::OnDemand,
    );
    let process = factory(&node, &module, Duration::from_millis(150))
        .start(MiniAppServiceLaunch {
            spec: spec.clone(),
            host_generation: 1,
        })
        .await
        .expect("start timeout Service process");
    let timeout = invoke(
        &process,
        &spec,
        1,
        "call-timeout",
        "hang_forever",
        json!({}),
        MiniAppCallCancellation::default(),
    )
    .await
    .expect_err("watchdog must fail a hung Service");
    assert!(matches!(timeout, MiniAppServiceProcessError::Crashed(_)));
    process.stop().await;
}
