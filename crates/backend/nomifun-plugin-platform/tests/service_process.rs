#[allow(dead_code)]
#[path = "../src/data_root.rs"]
mod data_root;
pub use data_root::*;

#[allow(dead_code)]
#[path = "../src/service_process.rs"]
mod service_process;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_process_runtime::probe_process_identity;
use nomifun_agent_contracts::{DigestHex, PluginId};
use serde_json::{Value as JsonValue, json};
use service_process::{
    NodePluginServiceProcess, NodePluginServiceProcessFactory, PluginSecret,
    PluginServiceActionsPort, PluginServiceCancellation, PluginServiceError, PluginServiceFence,
    PluginServiceGrants, PluginServiceHostPort, PluginServiceInvocation, PluginServiceLaunch,
    PluginServicePortError, PluginServicePorts, PluginServiceProcess, PluginServiceProcessLimits,
    PluginServiceSecretsPort,
};

const SERVICE_SOURCE: &str = r#"
import { spawn } from "node:child_process";

export async function activate(ctx) {
  await ctx.storage.kv.set("activation", {
    pluginId: ctx.pluginId,
    artifactDigest: ctx.artifactDigest,
    dataGeneration: ctx.dataGeneration,
    preview: ctx.preview,
  });
  return {
    async invoke(action, input) {
      if (action === "echo") return { input, pluginId: ctx.pluginId };
      if (action === "context") {
        return {
          pluginId: ctx.pluginId,
          artifactDigest: ctx.artifactDigest,
          dataGeneration: ctx.dataGeneration,
          preview: ctx.preview,
          config: ctx.config.get(),
          secret: await ctx.secrets.get("api_key"),
          host: await ctx.host.invoke("desktop.files.open", { path: input.path }),
          action: await ctx.actions.invoke("plugin:other/action", { value: input.value }),
        };
      }
      if (action === "storage") {
        await ctx.storage.kv.set("service-key", { value: input.value });
        const kv = await ctx.storage.kv.read("service-key");
        await ctx.storage.db.execute("CREATE TABLE IF NOT EXISTS entries (id INTEGER PRIMARY KEY, value TEXT NOT NULL)");
        await ctx.storage.db.execute("INSERT INTO entries (value) VALUES (?1)", [input.value]);
        const db = await ctx.storage.db.query("SELECT value FROM entries ORDER BY id");
        await ctx.storage.files.write("nested/value.txt", input.value);
        const file = (await ctx.storage.files.read("nested/value.txt")).toString("utf8");
        await ctx.cache.set("cached", input.value, 10000);
        const cached = await ctx.cache.get("cached");
        return { kv, db, file, cached };
      }
      if (action === "environment") return Object.keys(process.env).sort();
      if (action === "wait") {
        await new Promise((resolve, reject) => {
          if (ctx.signal.aborted) {
            const error = new Error("canceled"); error.name = "AbortError"; reject(error); return;
          }
          ctx.signal.addEventListener("abort", () => {
            const error = new Error("canceled"); error.name = "AbortError"; reject(error);
          }, { once: true });
        });
        return null;
      }
      if (action === "timeout") await new Promise(() => {});
      if (action === "crash") {
        process.exit(81);
        await new Promise(() => {});
      }
      if (action === "spawn-child") {
        const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
          stdio: "ignore",
        });
        return { pid: child.pid };
      }
      throw new Error("unsupported action");
    },
    async deactivate() {
      await ctx.storage.kv.set("deactivated", true);
    },
  };
}
"#;

#[derive(Default)]
struct FixturePorts;

#[async_trait]
impl PluginServiceSecretsPort for FixturePorts {
    async fn get(
        &self,
        _plugin_id: &PluginId,
        slot: &str,
        credential_id: &str,
        _preview: bool,
    ) -> Result<Option<PluginSecret>, PluginServicePortError> {
        assert_eq!(slot, "api_key");
        assert_eq!(credential_id, "provider:0199aa00-0000-7000-8000-000000000001");
        Ok(Some(PluginSecret::new("fixture-secret")))
    }
}

#[async_trait]
impl PluginServiceHostPort for FixturePorts {
    async fn invoke(
        &self,
        _plugin_id: &PluginId,
        capability: &str,
        input: JsonValue,
        _preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        assert!(!cancellation.is_canceled());
        Ok(json!({"capability": capability, "input": input}))
    }
}

#[async_trait]
impl PluginServiceActionsPort for FixturePorts {
    async fn invoke(
        &self,
        _caller_plugin_id: &PluginId,
        action: &str,
        input: JsonValue,
        call_chain: Vec<String>,
        _preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        assert!(!cancellation.is_canceled());
        Ok(json!({"action": action, "input": input, "callChain": call_chain}))
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    manager: PluginDataRootManager,
    module_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let manager = PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap();
        let service = temp.path().join("artifact").join("service");
        fs::create_dir_all(&service).unwrap();
        let module_path = service.join("main.mjs");
        fs::write(&module_path, SERVICE_SOURCE).unwrap();
        Self {
            _temp: temp,
            manager,
            module_path,
        }
    }

    fn generation(&self, suffix: &str) -> PluginDataRootHandle {
        self.manager
            .create_empty_generation(plugin(suffix), DataGeneration::new("generation-1").unwrap())
            .unwrap()
    }
}

fn plugin(suffix: &str) -> PluginId {
    PluginId::from(format!("0199bb00-0000-7000-8000-{suffix:0>12}"))
}

fn node_executable() -> Option<PathBuf> {
    which::which("node")
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
}

fn ports() -> PluginServicePorts {
    let ports = Arc::new(FixturePorts);
    PluginServicePorts {
        secrets: ports.clone(),
        host: ports.clone(),
        actions: ports,
    }
}

fn grants() -> PluginServiceGrants {
    PluginServiceGrants {
        secret_slots: BTreeSet::from(["api_key".into()]),
        host_capabilities: BTreeSet::from(["desktop.files.open".into()]),
        allow_action_invoke: true,
    }
}

fn fence(root: &PluginDataRootHandle, generation: u64, digest: char) -> PluginServiceFence {
    PluginServiceFence {
        plugin_id: root.plugin_id().clone(),
        artifact_digest: DigestHex::from(digest.to_string().repeat(64)),
        data_generation: root.generation().clone(),
        process_generation: generation,
    }
}

fn launch(
    fixture: &Fixture,
    root: PluginDataRootHandle,
    generation: u64,
    digest: char,
    preview: bool,
) -> PluginServiceLaunch {
    PluginServiceLaunch {
        fence: fence(&root, generation, digest),
        module_path: fixture.module_path.clone(),
        data_root: root,
        config: json!({"theme": "dark"}),
        credential_bindings: BTreeMap::from([(
            "api_key".into(),
            "provider:0199aa00-0000-7000-8000-000000000001".into(),
        )]),
        grants: grants(),
        preview,
    }
}

async fn invoke(
    process: &Arc<NodePluginServiceProcess>,
    action: &str,
    input: JsonValue,
) -> Result<JsonValue, PluginServiceError> {
    process
        .invoke(
            PluginServiceInvocation {
                fence: process.fence().clone(),
                action: action.into(),
                input,
                call_chain: vec!["plugin:caller/root".into()],
            },
            PluginServiceCancellation::default(),
        )
        .await
}

fn factory(
    node: &Path,
    limits: Option<PluginServiceProcessLimits>,
) -> NodePluginServiceProcessFactory {
    let factory = NodePluginServiceProcessFactory::new(node)
        .unwrap()
        .with_ports(ports());
    match limits {
        Some(limits) => factory.with_limits(limits).unwrap(),
        None => factory,
    }
}

#[tokio::test]
async fn real_node_receives_unified_context_and_round_trips_all_host_managed_storage() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping real service process coverage");
        return;
    };
    let fixture = Fixture::new();
    let root = fixture.generation("1");
    let process = factory(&node, None)
        .spawn(launch(&fixture, root.clone(), 1, 'a', false))
        .await
        .unwrap();

    let activated = root.storage().kv_get("activation").unwrap().value.unwrap();
    assert_eq!(activated["pluginId"], root.plugin_id().as_ref());
    assert_eq!(activated["preview"], false);

    let context = invoke(&process, "context", json!({"path":"note.txt","value":7}))
        .await
        .unwrap();
    assert_eq!(context["pluginId"], root.plugin_id().as_ref());
    assert_eq!(context["artifactDigest"], "a".repeat(64));
    assert_eq!(context["dataGeneration"], "generation-1");
    assert_eq!(context["preview"], false);
    assert_eq!(context["config"], json!({"theme":"dark"}));
    assert_eq!(context["secret"], "fixture-secret");
    assert_eq!(context["host"]["capability"], "desktop.files.open");
    assert_eq!(context["action"]["action"], "plugin:other/action");
    assert_eq!(context["action"]["callChain"][0], "plugin:caller/root");

    let storage = invoke(&process, "storage", json!({"value":"persisted"}))
        .await
        .unwrap();
    assert_eq!(storage["kv"]["value"], json!({"value":"persisted"}));
    assert_eq!(storage["db"]["rows"][0][0], "persisted");
    assert_eq!(storage["file"], "persisted");
    assert_eq!(storage["cached"], "persisted");

    let environment = invoke(&process, "environment", json!({}))
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    // Node on macOS adds this Core Foundation process setting even with an
    // empty inherited environment (`env -i node` has the same behavior).
    let allowed = [
        "COMSPEC",
        "HOME",
        "LANG",
        "LC_ALL",
        "NO_COLOR",
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "TZ",
        "USERPROFILE",
        "WINDIR",
    ]
    .into_iter()
    .chain(cfg!(target_os = "macos").then_some("__CF_USER_TEXT_ENCODING"))
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    assert!(environment.is_subset(&allowed), "unexpected ambient environment: {environment:?}");
    assert!(
        environment
            .iter()
            .all(|key| !key.contains("TOKEN") && !key.contains("SECRET") && !key.contains("PASSWORD"))
    );

    process.stop().await.unwrap();
    assert_eq!(root.storage().kv_get("deactivated").unwrap().value, Some(json!(true)));
}

#[tokio::test]
async fn cross_plugin_action_dispatch_requires_the_generic_actions_invoke_grant() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping real service process coverage");
        return;
    };
    let fixture = Fixture::new();
    let root = fixture.generation("8");
    let mut denied_launch = launch(&fixture, root, 1, '8', false);
    denied_launch.grants.allow_action_invoke = false;
    let process = factory(&node, None).spawn(denied_launch).await.unwrap();

    assert_eq!(
        invoke(&process, "context", json!({"path":"note.txt","value":7})).await,
        Err(PluginServiceError::Rejected {
            code: "service_invocation_failed".into(),
        })
    );
    process.stop().await.unwrap();
}

#[tokio::test]
async fn preview_uses_the_same_service_and_storage_contract_without_touching_the_live_generation() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping real service process coverage");
        return;
    };
    let fixture = Fixture::new();
    let live = fixture.generation("2");
    live.storage().kv_set("service-key", &json!({"value":"live"})).unwrap();
    let preview = fixture.manager.clone_preview(&live, "preview-session").unwrap();
    let preview_handle = preview.handle().clone();
    let process = factory(&node, None)
        .spawn(launch(&fixture, preview_handle.clone(), 1, 'b', true))
        .await
        .unwrap();

    let context = invoke(&process, "context", json!({"path":"x","value":1}))
        .await
        .unwrap();
    assert_eq!(context["preview"], true);
    invoke(&process, "storage", json!({"value":"preview"}))
        .await
        .unwrap();
    assert_eq!(
        preview_handle.storage().kv_get("service-key").unwrap().value,
        Some(json!({"value":"preview"}))
    );
    assert_eq!(
        live.storage().kv_get("service-key").unwrap().value,
        Some(json!({"value":"live"}))
    );
    process.stop().await.unwrap();
    let preview_path = preview_handle.path().to_path_buf();
    preview.destroy().unwrap();
    assert!(!preview_path.exists());
}

#[tokio::test]
async fn cancellation_is_request_scoped_and_timeout_terminates_only_that_plugin_process() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping real service process coverage");
        return;
    };
    let fixture = Fixture::new();
    let root = fixture.generation("3");
    let process = factory(&node, None)
        .spawn(launch(&fixture, root, 1, 'c', false))
        .await
        .unwrap();
    let cancellation = PluginServiceCancellation::default();
    let waiter = {
        let process = process.clone();
        let cancellation_for_call = cancellation.clone();
        tokio::spawn(async move {
            process
                .invoke(
                    PluginServiceInvocation {
                        fence: process.fence().clone(),
                        action: "wait".into(),
                        input: JsonValue::Null,
                        call_chain: vec!["plugin:caller/wait".into()],
                    },
                    cancellation_for_call,
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancellation.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), waiter)
            .await
            .unwrap()
            .unwrap(),
        Err(PluginServiceError::Canceled)
    );
    assert_eq!(invoke(&process, "echo", json!("still-running")).await.unwrap()["input"], "still-running");
    process.stop().await.unwrap();

    let short = PluginServiceProcessLimits {
        request_timeout: Duration::from_millis(150),
        cancellation_poll_interval: Duration::from_millis(10),
        ..PluginServiceProcessLimits::default()
    };
    let timeout_root = fixture.generation("4");
    let timeout_process = factory(&node, Some(short))
        .spawn(launch(&fixture, timeout_root, 1, 'd', false))
        .await
        .unwrap();
    assert_eq!(
        invoke(&timeout_process, "timeout", JsonValue::Null).await,
        Err(PluginServiceError::TimedOut)
    );
    wait_terminal(&timeout_process).await;
}

#[tokio::test]
async fn a_crashing_plugin_does_not_restart_or_break_another_plugin_process() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping real service process coverage");
        return;
    };
    let fixture = Fixture::new();
    let first_root = fixture.generation("5");
    let second_root = fixture.generation("6");
    let factory = factory(&node, None);
    let first = factory
        .spawn(launch(&fixture, first_root, 1, 'e', false))
        .await
        .unwrap();
    let second = factory
        .spawn(launch(&fixture, second_root, 1, 'f', false))
        .await
        .unwrap();
    let second_pid = second.process_id();

    assert!(matches!(
        invoke(&first, "crash", JsonValue::Null).await,
        Err(PluginServiceError::Crashed(_))
    ));
    wait_terminal(&first).await;
    assert_eq!(second.process_id(), second_pid);
    assert_eq!(invoke(&second, "echo", json!(42)).await.unwrap()["input"], 42);
    second.stop().await.unwrap();
}

#[tokio::test]
async fn managed_stop_reaps_a_service_spawned_process_tree() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping real service process coverage");
        return;
    };
    let fixture = Fixture::new();
    let root = fixture.generation("7");
    let process = factory(&node, None)
        .spawn(launch(&fixture, root, 1, '7', false))
        .await
        .unwrap();
    let child_pid = invoke(&process, "spawn-child", JsonValue::Null)
        .await
        .unwrap()["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .unwrap();
    assert!(probe_process_identity(child_pid).unwrap().is_some());
    process.stop().await.unwrap();
    wait_process_gone(child_pid).await;
}

async fn wait_terminal(process: &Arc<NodePluginServiceProcess>) {
    for _ in 0..100 {
        if process.terminal_result().is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("service process did not become terminal");
}

async fn wait_process_gone(pid: u32) {
    for _ in 0..150 {
        if probe_process_identity(pid).unwrap().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("service child process {pid} survived managed shutdown");
}
