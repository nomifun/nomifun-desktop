//! One isolated Node process for one Unified Plugin service.
//!
//! This module owns only in-memory execution facts. Durable activation state
//! remains in Plugin Core; a process is fenced by the stable Plugin identity,
//! immutable Artifact digest, and exact DataRoot generation supplied by the
//! activation coordinator.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_contracts::{DigestHex, PluginId};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    DataGeneration, DataRootKind, KvCompareAndSwap, PluginDataRootHandle, PluginFileEntry,
    PluginQueryResult, PluginSqlStatement, PluginSqlValue, PluginStorage, ScopedPluginCache,
};

const SERVICE_PROTOCOL_VERSION: &str = "1.0.0";
const SERVICE_ENTRYPOINT: &str = "main.mjs";
const MAX_MODULE_BYTES: u64 = 256 * 1024 * 1024;

const SERVICE_PROCESS_SCRIPT: &str = r#"
import { AsyncLocalStorage } from "node:async_hooks";
import readline from "node:readline";
import { pathToFileURL } from "node:url";

const bootstrap = JSON.parse(Buffer.from(process.argv.at(-1) ?? "", "hex").toString("utf8"));
if (
  bootstrap.protocolVersion !== "1.0.0" ||
  typeof bootstrap.pluginId !== "string" ||
  typeof bootstrap.artifactDigest !== "string" ||
  typeof bootstrap.dataGeneration !== "string" ||
  !Number.isSafeInteger(bootstrap.processGeneration) ||
  bootstrap.processGeneration <= 0 ||
  typeof bootstrap.modulePath !== "string"
) throw new Error("invalid Unified Plugin service bootstrap");

const protocolVersion = bootstrap.protocolVersion;
const processGeneration = bootstrap.processGeneration;
const invocation = new AsyncLocalStorage();
const activationController = new AbortController();
const activeRequests = new Map();
const hostRequests = new Map();
let hostSequence = 0;
let writeChain = Promise.resolve();
let service;
let shuttingDown = false;

function writeFrame(frame) {
  const line = `${JSON.stringify(frame)}\n`;
  writeChain = writeChain.then(() => new Promise((resolve, reject) => {
    process.stdout.write(line, error => error ? reject(error) : resolve());
  }));
  return writeChain;
}

function currentSignal() {
  return invocation.getStore()?.signal ?? activationController.signal;
}

const dynamicSignal = Object.freeze({
  get aborted() { return currentSignal().aborted; },
  get reason() { return currentSignal().reason; },
  addEventListener(...args) { return currentSignal().addEventListener(...args); },
  removeEventListener(...args) { return currentSignal().removeEventListener(...args); },
  throwIfAborted() { return currentSignal().throwIfAborted(); },
});

function hostError(code) {
  const error = new Error("Plugin Host request failed");
  error.code = String(code || "host_request_failed");
  return error;
}

function requestHost(operation, payload = {}) {
  const requestId = `${processGeneration}-host-${++hostSequence}`;
  const parentRequestId = invocation.getStore()?.requestId ?? null;
  const signal = currentSignal();
  return new Promise((resolve, reject) => {
    const abort = () => {
      const pending = hostRequests.get(requestId);
      if (!pending) return;
      hostRequests.delete(requestId);
      pending.reject(hostError("service_call_canceled"));
    };
    if (signal.aborted) {
      reject(hostError("service_call_canceled"));
      return;
    }
    signal.addEventListener("abort", abort, { once: true });
    hostRequests.set(requestId, { resolve, reject, signal, abort });
    void writeFrame({
      kind: "host_request", protocolVersion, processGeneration,
      requestId, parentRequestId, operation, payload,
    }).catch(() => abort());
  });
}

function resolveHost(frame) {
  const pending = hostRequests.get(frame.requestId);
  if (!pending) return;
  hostRequests.delete(frame.requestId);
  pending.signal.removeEventListener("abort", pending.abort);
  if (frame.outcome === "success") pending.resolve(frame.value ?? null);
  else pending.reject(hostError(frame.error?.code));
}

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`invalid ${label}`);
  return value;
}

function encodeBytes(value) {
  if (typeof value === "string") return Buffer.from(value, "utf8").toString("hex");
  if (value instanceof ArrayBuffer) return Buffer.from(value).toString("hex");
  if (ArrayBuffer.isView(value)) return Buffer.from(value.buffer, value.byteOffset, value.byteLength).toString("hex");
  throw new Error("files.write expects a string, ArrayBuffer, or typed array");
}

const context = Object.freeze({
  pluginId: bootstrap.pluginId,
  artifactDigest: bootstrap.artifactDigest,
  dataGeneration: bootstrap.dataGeneration,
  preview: bootstrap.preview === true,
  signal: dynamicSignal,
  storage: Object.freeze({
    kv: Object.freeze({
      async get(key) { return (await requestHost("kv.get", { key: requireString(key, "KV key") })).value ?? null; },
      async read(key) { return requestHost("kv.read", { key: requireString(key, "KV key") }); },
      async set(key, value) { return requestHost("kv.set", { key: requireString(key, "KV key"), value }); },
      async delete(key) { return requestHost("kv.delete", { key: requireString(key, "KV key") }); },
      async compareAndSwap(key, expectedRevision, value) {
        return requestHost("kv.compareAndSwap", {
          key: requireString(key, "KV key"), expectedRevision: expectedRevision ?? null,
          hasValue: value !== undefined && value !== null, value: value ?? null,
        });
      },
    }),
    db: Object.freeze({
      async query(sql, parameters = []) { return requestHost("db.query", { sql: requireString(sql, "SQL"), parameters }); },
      async execute(sql, parameters = []) { return requestHost("db.execute", { sql: requireString(sql, "SQL"), parameters }); },
      async batch(statements) { return requestHost("db.batch", { statements }); },
    }),
    files: Object.freeze({
      async read(path) {
        const result = await requestHost("files.read", { path: requireString(path, "file path") });
        return Buffer.from(result.bytesHex, "hex");
      },
      async write(path, value) { return requestHost("files.write", { path: requireString(path, "file path"), bytesHex: encodeBytes(value) }); },
      async list(path = null) { return requestHost("files.list", { path }); },
      async delete(path) { return requestHost("files.delete", { path: requireString(path, "file path") }); },
    }),
  }),
  cache: Object.freeze({
    async get(key) { return (await requestHost("cache.get", { key: requireString(key, "cache key") })).value ?? null; },
    async set(key, value, ttlMs = null) { return requestHost("cache.set", { key: requireString(key, "cache key"), value, ttlMs }); },
    async delete(key) { return requestHost("cache.delete", { key: requireString(key, "cache key") }); },
  }),
  config: Object.freeze({ get() { return structuredClone(bootstrap.config); } }),
  secrets: Object.freeze({ async get(slot) { return (await requestHost("secrets.get", { slot: requireString(slot, "secret slot") })).value; } }),
  host: Object.freeze({ async invoke(capability, input = {}) { return requestHost("host.invoke", { capability: requireString(capability, "Host capability"), input }); } }),
  actions: Object.freeze({ async invoke(action, input = {}) { return requestHost("actions.invoke", { action: requireString(action, "Action"), input }); } }),
});

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity, terminal: false });
lines.on("line", line => {
  let frame;
  try { frame = JSON.parse(line); }
  catch { process.exitCode = 70; return; }
  void dispatch(frame).catch(() => { process.exitCode = 71; });
});
lines.on("close", () => { if (!shuttingDown) process.exitCode = 72; });

const moduleUrl = pathToFileURL(bootstrap.modulePath);
moduleUrl.searchParams.set("artifact", bootstrap.artifactDigest);
const imported = await import(moduleUrl.href);
if (typeof imported.activate !== "function") throw new Error("service/main.mjs must export activate(ctx)");
service = await imported.activate(context);
if (!service || typeof service !== "object" || typeof service.invoke !== "function") {
  throw new Error("activate(ctx) must return { invoke(action, input) }");
}

await writeFrame({
  kind: "hello", protocolVersion, processGeneration, processId: process.pid,
  pluginId: bootstrap.pluginId, artifactDigest: bootstrap.artifactDigest,
  dataGeneration: bootstrap.dataGeneration,
});

async function dispatch(frame) {
  if (!frame || frame.protocolVersion !== protocolVersion || frame.processGeneration !== processGeneration) {
    throw new Error("service frame fence mismatch");
  }
  if (frame.kind === "host_response") { resolveHost(frame); return; }
  if (frame.kind === "control" && frame.operation === "cancel") {
    activeRequests.get(frame.targetRequestId)?.abort();
    await writeFrame({ kind: "response", protocolVersion, processGeneration, requestId: frame.requestId, outcome: "ack" });
    return;
  }
  if (frame.kind === "control" && frame.operation === "shutdown") {
    shuttingDown = true;
    for (const controller of activeRequests.values()) controller.abort();
    if (typeof service?.deactivate === "function") await service.deactivate();
    activationController.abort();
    for (const pending of hostRequests.values()) pending.reject(hostError("service_stopping"));
    hostRequests.clear();
    await writeFrame({ kind: "response", protocolVersion, processGeneration, requestId: frame.requestId, outcome: "ack" });
    lines.close();
    return;
  }
  if (frame.kind !== "invoke" || typeof frame.requestId !== "string" || typeof frame.action !== "string") {
    throw new Error("unsupported service request");
  }
  const controller = new AbortController();
  activeRequests.set(frame.requestId, controller);
  try {
    const value = await invocation.run(
      { requestId: frame.requestId, signal: controller.signal },
      () => service.invoke(frame.action, structuredClone(frame.input ?? null)),
    );
    await writeFrame({ kind: "response", protocolVersion, processGeneration, requestId: frame.requestId, outcome: "success", value: value ?? null });
  } catch (error) {
    await writeFrame({
      kind: "response", protocolVersion, processGeneration, requestId: frame.requestId,
      outcome: "failure", error: { code: error?.name === "AbortError" || controller.signal.aborted ? "service_call_canceled" : "service_invocation_failed" },
    });
  } finally { activeRequests.delete(frame.requestId); }
}

process.on("uncaughtException", () => { process.exitCode = 73; });
process.on("unhandledRejection", () => { process.exitCode = 74; });
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginServiceFence {
    pub plugin_id: PluginId,
    pub artifact_digest: DigestHex,
    pub data_generation: DataGeneration,
    pub process_generation: u64,
}

impl PluginServiceFence {
    pub fn validate(&self) -> PluginServiceResult<()> {
        if self.plugin_id.as_ref().is_empty()
            || !valid_digest(self.artifact_digest.as_ref())
            || self.process_generation == 0
        {
            return Err(PluginServiceError::InvalidConfiguration(
                "service fence is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginServiceGrants {
    pub secret_slots: BTreeSet<String>,
    pub host_capabilities: BTreeSet<String>,
    pub allow_action_invoke: bool,
}

#[derive(Clone, Debug)]
pub struct PluginServiceLaunch {
    pub fence: PluginServiceFence,
    pub module_path: PathBuf,
    pub data_root: PluginDataRootHandle,
    pub config: JsonValue,
    pub credential_bindings: BTreeMap<String, String>,
    pub grants: PluginServiceGrants,
    pub preview: bool,
}

impl PluginServiceLaunch {
    fn validate(&self) -> PluginServiceResult<()> {
        self.fence.validate()?;
        if self.data_root.plugin_id() != &self.fence.plugin_id
            || self.data_root.generation() != &self.fence.data_generation
            || (!self.preview && self.data_root.kind() != DataRootKind::Generation)
            || (self.preview
                && !matches!(
                    self.data_root.kind(),
                    DataRootKind::Preview | DataRootKind::Staging
                ))
            || !self.config.is_object()
            || self
                .credential_bindings
                .keys()
                .any(|slot| !self.grants.secret_slots.contains(slot))
        {
            return Err(PluginServiceError::InvalidConfiguration(
                "service launch does not match its Plugin, DataRoot, mode, or config".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginServiceInvocation {
    pub fence: PluginServiceFence,
    pub action: String,
    pub input: JsonValue,
    /// Host-owned stable Action chain. It is never exposed as mutable Plugin
    /// context and is carried only so nested `ctx.actions.invoke` calls retain
    /// the registry's recursion and depth fences.
    pub call_chain: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PluginServiceCancellation {
    canceled: Arc<AtomicBool>,
}

impl PluginServiceCancellation {
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
pub struct PluginSecret(String);

impl PluginSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PluginSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PluginSecret([REDACTED])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginServicePortError {
    code: String,
}

impl PluginServicePortError {
    pub fn new(code: impl Into<String>) -> Self {
        let code = code.into();
        let code = if code.is_empty()
            || code.len() > 128
            || !code.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            "port_unavailable".into()
        } else {
            code
        };
        Self { code }
    }

    fn code(&self) -> &str {
        &self.code
    }
}

#[async_trait]
pub trait PluginServiceSecretsPort: Send + Sync {
    async fn get(
        &self,
        plugin_id: &PluginId,
        slot: &str,
        credential_id: &str,
        preview: bool,
    ) -> Result<Option<PluginSecret>, PluginServicePortError>;
}

#[async_trait]
pub trait PluginServiceHostPort: Send + Sync {
    async fn invoke(
        &self,
        plugin_id: &PluginId,
        capability: &str,
        input: JsonValue,
        preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError>;
}

#[async_trait]
pub trait PluginServiceActionsPort: Send + Sync {
    async fn invoke(
        &self,
        caller_plugin_id: &PluginId,
        action: &str,
        input: JsonValue,
        call_chain: Vec<String>,
        preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError>;
}

#[derive(Clone)]
pub struct PluginServicePorts {
    pub secrets: Arc<dyn PluginServiceSecretsPort>,
    pub host: Arc<dyn PluginServiceHostPort>,
    pub actions: Arc<dyn PluginServiceActionsPort>,
}

impl std::fmt::Debug for PluginServicePorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("PluginServicePorts").finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub struct DenyPluginServicePorts;

#[async_trait]
impl PluginServiceSecretsPort for DenyPluginServicePorts {
    async fn get(
        &self,
        _plugin_id: &PluginId,
        _slot: &str,
        _credential_id: &str,
        _preview: bool,
    ) -> Result<Option<PluginSecret>, PluginServicePortError> {
        Err(PluginServicePortError::new("secret_unavailable"))
    }
}

#[async_trait]
impl PluginServiceHostPort for DenyPluginServicePorts {
    async fn invoke(
        &self,
        _plugin_id: &PluginId,
        _capability: &str,
        _input: JsonValue,
        _preview: bool,
        _cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        Err(PluginServicePortError::new("host_capability_denied"))
    }
}

#[async_trait]
impl PluginServiceActionsPort for DenyPluginServicePorts {
    async fn invoke(
        &self,
        _caller_plugin_id: &PluginId,
        _action: &str,
        _input: JsonValue,
        _call_chain: Vec<String>,
        _preview: bool,
        _cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        Err(PluginServicePortError::new("action_denied"))
    }
}

impl Default for PluginServicePorts {
    fn default() -> Self {
        let deny = Arc::new(DenyPluginServicePorts);
        Self {
            secrets: deny.clone(),
            host: deny.clone(),
            actions: deny,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginServiceProcessLimits {
    pub hello_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_frame_bytes: usize,
    pub command_queue_capacity: usize,
    pub cancellation_poll_interval: Duration,
}

impl Default for PluginServiceProcessLimits {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(5),
            max_frame_bytes: 4 * 1024 * 1024,
            command_queue_capacity: 256,
            cancellation_poll_interval: Duration::from_millis(10),
        }
    }
}

impl PluginServiceProcessLimits {
    fn validate(&self) -> PluginServiceResult<()> {
        if self.hello_timeout.is_zero()
            || self.request_timeout.is_zero()
            || self.shutdown_timeout.is_zero()
            || self.max_frame_bytes < 1_024
            || self.command_queue_capacity == 0
            || self.cancellation_poll_interval.is_zero()
        {
            return Err(PluginServiceError::InvalidConfiguration(
                "service process limits must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PluginServiceError {
    #[error("invalid Plugin service configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Plugin service process could not start: {0}")]
    Spawn(String),
    #[error("Plugin service protocol failed: {0}")]
    Protocol(String),
    #[error("Plugin service command queue is full")]
    QueueFull,
    #[error("Plugin service call was canceled")]
    Canceled,
    #[error("Plugin service call timed out")]
    TimedOut,
    #[error("Plugin service rejected the call: {code}")]
    Rejected { code: String },
    #[error("Plugin service process failed: {0}")]
    Crashed(String),
    #[error("Plugin service invocation used a stale fence")]
    StaleFence,
}

pub type PluginServiceResult<T> = Result<T, PluginServiceError>;

#[async_trait]
pub trait PluginServiceProcess: Send + Sync {
    fn fence(&self) -> &PluginServiceFence;
    fn process_id(&self) -> u32;
    async fn invoke(
        &self,
        invocation: PluginServiceInvocation,
        cancellation: PluginServiceCancellation,
    ) -> PluginServiceResult<JsonValue>;
    async fn stop(&self) -> PluginServiceResult<()>;
    fn terminal_result(&self) -> Option<PluginServiceResult<()>>;
}

pub struct NodePluginServiceProcessFactory {
    node_executable: PathBuf,
    ports: PluginServicePorts,
    limits: PluginServiceProcessLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginModuleKind {
    Service,
    Migration,
}

impl std::fmt::Debug for NodePluginServiceProcessFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodePluginServiceProcessFactory")
            .field("node_executable", &self.node_executable)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl NodePluginServiceProcessFactory {
    pub fn new(node_executable: impl Into<PathBuf>) -> PluginServiceResult<Self> {
        let requested = node_executable.into();
        if !requested.is_absolute() {
            return Err(PluginServiceError::InvalidConfiguration(
                "Node executable must be absolute".into(),
            ));
        }
        let node_executable = fs::canonicalize(&requested)
            .map_err(|_| PluginServiceError::InvalidConfiguration("Node unavailable".into()))?;
        require_regular_file(&node_executable, MAX_MODULE_BYTES)?;
        Ok(Self {
            node_executable,
            ports: PluginServicePorts::default(),
            limits: PluginServiceProcessLimits::default(),
        })
    }

    pub fn with_ports(mut self, ports: PluginServicePorts) -> Self {
        self.ports = ports;
        self
    }

    pub fn with_limits(mut self, limits: PluginServiceProcessLimits) -> PluginServiceResult<Self> {
        limits.validate()?;
        self.limits = limits;
        Ok(self)
    }

    pub async fn spawn(
        &self,
        launch: PluginServiceLaunch,
    ) -> PluginServiceResult<Arc<NodePluginServiceProcess>> {
        self.spawn_inner(launch, PluginModuleKind::Service).await
    }

    pub(crate) async fn spawn_migration(
        &self,
        launch: PluginServiceLaunch,
    ) -> PluginServiceResult<Arc<NodePluginServiceProcess>> {
        self.spawn_inner(launch, PluginModuleKind::Migration).await
    }

    async fn spawn_inner(
        &self,
        launch: PluginServiceLaunch,
        module_kind: PluginModuleKind,
    ) -> PluginServiceResult<Arc<NodePluginServiceProcess>> {
        self.limits.validate()?;
        launch.validate()?;
        require_regular_file(&self.node_executable, MAX_MODULE_BYTES)?;
        let module_path = match module_kind {
            PluginModuleKind::Service => validate_module_path(&launch.module_path)?,
            PluginModuleKind::Migration => {
                validate_migration_module_path(&launch.module_path, &launch.data_root)?
            }
        };
        let bootstrap = ServiceBootstrap {
            protocol_version: SERVICE_PROTOCOL_VERSION,
            plugin_id: launch.fence.plugin_id.as_ref(),
            artifact_digest: launch.fence.artifact_digest.as_ref(),
            data_generation: launch.fence.data_generation.as_str(),
            process_generation: launch.fence.process_generation,
            module_path: module_path.to_str().ok_or_else(|| {
                PluginServiceError::InvalidConfiguration("service module path must be UTF-8".into())
            })?,
            preview: launch.preview,
            config: &launch.config,
        };
        let bootstrap = hex::encode(
            serde_json::to_vec(&bootstrap)
                .map_err(|_| PluginServiceError::InvalidConfiguration("bootstrap encoding failed".into()))?,
        );
        let mut builder = ChildProcessBuilder::new(&self.node_executable);
        builder
            .arg("--input-type=module")
            .arg("-e")
            .arg(SERVICE_PROCESS_SCRIPT)
            .arg(&bootstrap)
            .current_dir(module_path.parent().ok_or_else(|| {
                PluginServiceError::InvalidConfiguration("service module has no parent".into())
            })?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_plugin_service_environment(&mut builder);
        let mut process = builder
            .spawn_managed()
            .map_err(|_| PluginServiceError::Spawn("Node spawn failed".into()))?;
        let process_id = process
            .id()
            .ok_or_else(|| PluginServiceError::Spawn("Node process has no pid".into()))?;
        let mut stdin = process
            .stdin
            .take()
            .ok_or_else(|| PluginServiceError::Spawn("Node stdin unavailable".into()))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| PluginServiceError::Spawn("Node stdout unavailable".into()))?;
        let stderr = process
            .stderr
            .take()
            .ok_or_else(|| PluginServiceError::Spawn("Node stderr unavailable".into()))?;
        let context = ServiceContext::new(&launch, self.ports.clone());
        let mut reader = BufReader::new(stdout);
        let hello = match tokio::time::timeout(
            self.limits.hello_timeout,
            read_until_hello(
                &mut reader,
                &mut stdin,
                &launch.fence,
                process_id,
                &context,
                &self.limits,
            ),
        )
        .await
        {
            Ok(Ok(hello)) => hello,
            Ok(Err(error)) => {
                let _ = process.shutdown().await;
                return Err(error);
            }
            Err(_) => {
                let _ = process.shutdown().await;
                return Err(PluginServiceError::TimedOut);
            }
        };
        validate_hello(&hello, &launch.fence, process_id)?;

        let (reader_tx, reader_rx) = mpsc::channel(self.limits.command_queue_capacity);
        let reader_task = tokio::spawn(read_frames(reader, reader_tx, self.limits.max_frame_bytes));
        let stderr_task = tokio::spawn(drain_stderr(stderr));
        let (commands_tx, commands_rx) = mpsc::channel(self.limits.command_queue_capacity);
        let (completion_tx, completion_rx) = watch::channel(None);
        let actor = ServiceActor {
            process,
            stdin,
            reader: reader_rx,
            commands: commands_rx,
            command_sender: commands_tx.downgrade(),
            pending: BTreeMap::new(),
            host_pending: BTreeMap::new(),
            control_requests: BTreeSet::new(),
            shutdown_request: None,
            shutdown_deadline: None,
            context,
            fence: launch.fence.clone(),
            limits: self.limits.clone(),
            reader_task,
            stderr_task,
            completion: completion_tx,
            stopping: false,
        };
        tokio::spawn(actor.run());
        Ok(Arc::new(NodePluginServiceProcess {
            inner: Arc::new(ServiceProcessInner {
                fence: launch.fence,
                process_id,
                commands: commands_tx,
                limits: self.limits.clone(),
                stop_requested: AtomicBool::new(false),
                completion: completion_rx,
            }),
        }))
    }
}

const PLUGIN_SERVICE_ENVIRONMENT_ALLOWLIST: &[&str] = &[
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
];

fn apply_plugin_service_environment(builder: &mut ChildProcessBuilder) {
    builder.env_clear();
    for key in PLUGIN_SERVICE_ENVIRONMENT_ALLOWLIST {
        if let Some(value) = std::env::var_os(key) {
            builder.env(key, value);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceBootstrap<'a> {
    protocol_version: &'static str,
    plugin_id: &'a str,
    artifact_digest: &'a str,
    data_generation: &'a str,
    process_generation: u64,
    module_path: &'a str,
    preview: bool,
    config: &'a JsonValue,
}

#[derive(Clone)]
struct ServiceContext {
    plugin_id: PluginId,
    storage: PluginStorage,
    cache: ScopedPluginCache,
    grants: PluginServiceGrants,
    credential_bindings: BTreeMap<String, String>,
    preview: bool,
    ports: PluginServicePorts,
}

impl ServiceContext {
    fn new(launch: &PluginServiceLaunch, ports: PluginServicePorts) -> Arc<Self> {
        Arc::new(Self {
            plugin_id: launch.fence.plugin_id.clone(),
            storage: launch.data_root.storage(),
            cache: launch.data_root.cache(),
            grants: launch.grants.clone(),
            credential_bindings: launch.credential_bindings.clone(),
            preview: launch.preview,
            ports,
        })
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ServiceHello {
    kind: String,
    protocol_version: String,
    process_generation: u64,
    process_id: u32,
    plugin_id: String,
    artifact_digest: String,
    data_generation: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InvokeFrame<'a> {
    kind: &'static str,
    protocol_version: &'static str,
    process_generation: u64,
    request_id: &'a str,
    action: &'a str,
    input: &'a JsonValue,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ControlFrame<'a> {
    kind: &'static str,
    protocol_version: &'static str,
    process_generation: u64,
    request_id: &'a str,
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_request_id: Option<&'a str>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ResponseFrame {
    kind: String,
    protocol_version: String,
    process_generation: u64,
    request_id: String,
    outcome: String,
    value: Option<JsonValue>,
    error: Option<WireError>,
}

#[derive(Deserialize, Debug)]
struct WireError {
    code: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct HostRequestFrame {
    kind: String,
    protocol_version: String,
    process_generation: u64,
    request_id: String,
    parent_request_id: Option<String>,
    operation: String,
    payload: JsonValue,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostResponseFrame<'a> {
    kind: &'static str,
    protocol_version: &'static str,
    process_generation: u64,
    request_id: &'a str,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<HostWireError<'a>>,
}

#[derive(Serialize)]
struct HostWireError<'a> {
    code: &'a str,
}

enum InboundFrame {
    Response(ResponseFrame),
    Host(HostRequestFrame),
}

enum ReaderEvent {
    Frame(InboundFrame),
    Eof,
    Failed(String),
}

struct PendingInvocation {
    reply: oneshot::Sender<PluginServiceResult<JsonValue>>,
    cancellation: PluginServiceCancellation,
    call_chain: Vec<String>,
    deadline: Instant,
    cancel_sent: bool,
}

enum ActorCommand {
    Invoke {
        invocation: PluginServiceInvocation,
        cancellation: PluginServiceCancellation,
        reply: oneshot::Sender<PluginServiceResult<JsonValue>>,
    },
    HostCompleted {
        request_id: String,
        result: Result<JsonValue, PluginServicePortError>,
    },
    Stop,
}

enum ActorExit {
    Stopped,
    Failed(String),
}

struct ServiceActor {
    process: ManagedChildProcess,
    stdin: ChildStdin,
    reader: mpsc::Receiver<ReaderEvent>,
    commands: mpsc::Receiver<ActorCommand>,
    command_sender: mpsc::WeakSender<ActorCommand>,
    pending: BTreeMap<String, PendingInvocation>,
    host_pending: BTreeMap<String, PluginServiceCancellation>,
    control_requests: BTreeSet<String>,
    shutdown_request: Option<String>,
    shutdown_deadline: Option<Instant>,
    context: Arc<ServiceContext>,
    fence: PluginServiceFence,
    limits: PluginServiceProcessLimits,
    reader_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    completion: watch::Sender<Option<PluginServiceResult<()>>>,
    stopping: bool,
}

impl ServiceActor {
    async fn run(mut self) {
        let exit = self.run_loop().await;
        self.commands.close();
        let terminal = match &exit {
            ActorExit::Stopped => Ok(()),
            ActorExit::Failed(reason) => Err(PluginServiceError::Crashed(reason.clone())),
        };
        let pending_error = match &terminal {
            Ok(()) => PluginServiceError::Canceled,
            Err(error) => error.clone(),
        };
        for (_, pending) in std::mem::take(&mut self.pending) {
            pending.cancellation.cancel();
            let _ = pending.reply.send(Err(pending_error.clone()));
        }
        for cancellation in self.host_pending.values() {
            cancellation.cancel();
        }
        self.host_pending.clear();
        while let Ok(command) = self.commands.try_recv() {
            if let ActorCommand::Invoke { reply, .. } = command {
                let _ = reply.send(Err(pending_error.clone()));
            }
        }
        let _ = tokio::time::timeout(self.limits.shutdown_timeout, self.process.shutdown()).await;
        self.reader_task.abort();
        self.stderr_task.abort();
        let _ = self.completion.send(Some(terminal));
    }

    async fn run_loop(&mut self) -> ActorExit {
        let mut watchdog = tokio::time::interval(self.limits.cancellation_poll_interval);
        watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        return ActorExit::Failed("service command channel closed".into());
                    };
                    if let Some(exit) = self.handle_command(command).await { return exit; }
                }
                event = self.reader.recv() => {
                    match event {
                        Some(ReaderEvent::Frame(frame)) => {
                            if let Some(exit) = self.handle_frame(frame).await { return exit; }
                        }
                        Some(ReaderEvent::Eof) | None => {
                            if self.stopping { return ActorExit::Stopped; }
                            return ActorExit::Failed("service IPC reached EOF".into());
                        }
                        Some(ReaderEvent::Failed(reason)) => return ActorExit::Failed(reason),
                    }
                }
                _ = watchdog.tick() => {
                    if let Ok(Some(_)) = self.process.try_wait() {
                        if self.stopping { return ActorExit::Stopped; }
                        return ActorExit::Failed("service process exited".into());
                    }
                    if let Some(exit) = self.maintain().await { return exit; }
                }
            }
        }
    }

    async fn handle_command(&mut self, command: ActorCommand) -> Option<ActorExit> {
        match command {
            ActorCommand::Invoke { invocation, cancellation, reply } => {
                if self.stopping {
                    let _ = reply.send(Err(PluginServiceError::Crashed("service is stopping".into())));
                    return None;
                }
                if invocation.fence != self.fence {
                    let _ = reply.send(Err(PluginServiceError::StaleFence));
                    return None;
                }
                if cancellation.is_canceled() {
                    let _ = reply.send(Err(PluginServiceError::Canceled));
                    return None;
                }
                if invocation.action.is_empty() || invocation.action.len() > 128 {
                    let _ = reply.send(Err(PluginServiceError::InvalidConfiguration("invalid Action ID".into())));
                    return None;
                }
                if self.pending.len() >= self.limits.command_queue_capacity {
                    let _ = reply.send(Err(PluginServiceError::QueueFull));
                    return None;
                }
                let request_id = Uuid::now_v7().to_string();
                let frame = InvokeFrame {
                    kind: "invoke", protocol_version: SERVICE_PROTOCOL_VERSION,
                    process_generation: self.fence.process_generation,
                    request_id: &request_id, action: &invocation.action, input: &invocation.input,
                };
                if let Err(error) = self.write(&frame).await {
                    let _ = reply.send(Err(error.clone()));
                    return Some(ActorExit::Failed("service IPC write failed".into()));
                }
                self.pending.insert(request_id, PendingInvocation {
                    reply, cancellation, call_chain: invocation.call_chain,
                    deadline: Instant::now() + self.limits.request_timeout,
                    cancel_sent: false,
                });
            }
            ActorCommand::HostCompleted { request_id, result } => {
                if self.host_pending.remove(&request_id).is_none() { return None; }
                let (outcome, value, error) = match result {
                    Ok(value) => ("success", Some(value), None),
                    Err(error) => ("failure", None, Some(error)),
                };
                let error_code = error.as_ref().map(PluginServicePortError::code);
                let response = HostResponseFrame {
                    kind: "host_response", protocol_version: SERVICE_PROTOCOL_VERSION,
                    process_generation: self.fence.process_generation, request_id: &request_id,
                    outcome, value: value.as_ref(), error: error_code.map(|code| HostWireError { code }),
                };
                if self.write(&response).await.is_err() {
                    return Some(ActorExit::Failed("service Host response write failed".into()));
                }
            }
            ActorCommand::Stop => {
                if self.stopping { return None; }
                self.stopping = true;
                for pending in self.pending.values() { pending.cancellation.cancel(); }
                let ids = self.pending.keys().cloned().collect::<Vec<_>>();
                for request_id in ids { let _ = self.send_cancel(&request_id).await; }
                let shutdown_id = Uuid::now_v7().to_string();
                let frame = ControlFrame {
                    kind: "control", protocol_version: SERVICE_PROTOCOL_VERSION,
                    process_generation: self.fence.process_generation,
                    request_id: &shutdown_id, operation: "shutdown", target_request_id: None,
                };
                if self.write(&frame).await.is_err() {
                    return Some(ActorExit::Failed("service shutdown write failed".into()));
                }
                self.shutdown_request = Some(shutdown_id);
                self.shutdown_deadline = Some(Instant::now() + self.limits.shutdown_timeout);
            }
        }
        None
    }

    async fn handle_frame(&mut self, frame: InboundFrame) -> Option<ActorExit> {
        match frame {
            InboundFrame::Response(frame) => self.handle_response(frame).await,
            InboundFrame::Host(frame) => self.handle_host(frame).await,
        }
    }

    async fn handle_response(&mut self, frame: ResponseFrame) -> Option<ActorExit> {
        if frame.kind != "response"
            || frame.protocol_version != SERVICE_PROTOCOL_VERSION
            || frame.process_generation != self.fence.process_generation
        {
            return Some(ActorExit::Failed("service response fence mismatch".into()));
        }
        if self.shutdown_request.as_deref() == Some(&frame.request_id) {
            return if frame.outcome == "ack" {
                Some(ActorExit::Stopped)
            } else {
                Some(ActorExit::Failed("service shutdown was rejected".into()))
            };
        }
        if self.control_requests.remove(&frame.request_id) {
            return (frame.outcome != "ack")
                .then(|| ActorExit::Failed("service cancellation was rejected".into()));
        }
        let Some(pending) = self.pending.remove(&frame.request_id) else {
            return Some(ActorExit::Failed("service emitted an unknown response".into()));
        };
        let result = if pending.cancellation.is_canceled() {
            Err(PluginServiceError::Canceled)
        } else {
            match frame.outcome.as_str() {
                "success" => Ok(frame.value.unwrap_or(JsonValue::Null)),
                "failure" => {
                    let code = frame.error.map(|error| error.code).unwrap_or_else(|| "service_invocation_failed".into());
                    if code == "service_call_canceled" {
                        Err(PluginServiceError::Canceled)
                    } else {
                        Err(PluginServiceError::Rejected { code })
                    }
                }
                _ => Err(PluginServiceError::Protocol("invalid service response outcome".into())),
            }
        };
        let _ = pending.reply.send(result);
        None
    }

    async fn handle_host(&mut self, frame: HostRequestFrame) -> Option<ActorExit> {
        if frame.kind != "host_request"
            || frame.protocol_version != SERVICE_PROTOCOL_VERSION
            || frame.process_generation != self.fence.process_generation
            || frame.request_id.is_empty()
            || self.host_pending.contains_key(&frame.request_id)
        {
            return Some(ActorExit::Failed("service Host request is invalid".into()));
        }
        if self.host_pending.len() >= self.limits.command_queue_capacity {
            let error = PluginServicePortError::new("host_queue_full");
            let response = HostResponseFrame {
                kind: "host_response", protocol_version: SERVICE_PROTOCOL_VERSION,
                process_generation: self.fence.process_generation, request_id: &frame.request_id,
                outcome: "failure", value: None, error: Some(HostWireError { code: error.code() }),
            };
            if self.write(&response).await.is_err() {
                return Some(ActorExit::Failed("service Host queue response failed".into()));
            }
            return None;
        }
        let (cancellation, call_chain) = match frame.parent_request_id.as_deref() {
            Some(parent) => match self.pending.get(parent) {
                Some(parent) => (parent.cancellation.clone(), parent.call_chain.clone()),
                None => {
                    let error = PluginServicePortError::new("stale_parent_request");
                    let response = HostResponseFrame {
                        kind: "host_response", protocol_version: SERVICE_PROTOCOL_VERSION,
                        process_generation: self.fence.process_generation, request_id: &frame.request_id,
                        outcome: "failure", value: None, error: Some(HostWireError { code: error.code() }),
                    };
                    if self.write(&response).await.is_err() {
                        return Some(ActorExit::Failed("stale Host response failed".into()));
                    }
                    return None;
                }
            },
            None => (PluginServiceCancellation::default(), Vec::new()),
        };
        self.host_pending.insert(frame.request_id.clone(), cancellation.clone());
        let Some(sender) = self.command_sender.upgrade() else {
            return Some(ActorExit::Failed("service command channel closed".into()));
        };
        let timeout = if self.stopping {
            self.limits.shutdown_timeout
        } else {
            self.limits.request_timeout
        };
        spawn_host_request(
            Arc::clone(&self.context),
            frame,
            call_chain,
            cancellation,
            sender,
            timeout,
        );
        None
    }

    async fn maintain(&mut self) -> Option<ActorExit> {
        let now = Instant::now();
        if self
            .shutdown_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            return Some(ActorExit::Failed("service shutdown timed out".into()));
        }
        let canceled = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.cancellation.is_canceled() && !pending.cancel_sent)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for request_id in canceled {
            if let Some(pending) = self.pending.get_mut(&request_id) { pending.cancel_sent = true; }
            if self.send_cancel(&request_id).await.is_err() {
                return Some(ActorExit::Failed("service cancellation write failed".into()));
            }
        }
        let expired = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        if !expired.is_empty() {
            for request_id in expired {
                if let Some(pending) = self.pending.remove(&request_id) {
                    pending.cancellation.cancel();
                    let _ = pending.reply.send(Err(PluginServiceError::TimedOut));
                }
            }
            return Some(ActorExit::Failed("service action timed out".into()));
        }
        None
    }

    async fn send_cancel(&mut self, target: &str) -> PluginServiceResult<()> {
        let control_id = Uuid::now_v7().to_string();
        let frame = ControlFrame {
            kind: "control", protocol_version: SERVICE_PROTOCOL_VERSION,
            process_generation: self.fence.process_generation, request_id: &control_id,
            operation: "cancel", target_request_id: Some(target),
        };
        self.write(&frame).await?;
        self.control_requests.insert(control_id);
        Ok(())
    }

    async fn write<T: Serialize>(&mut self, value: &T) -> PluginServiceResult<()> {
        tokio::time::timeout(
            self.limits.request_timeout,
            write_json_line(&mut self.stdin, value, self.limits.max_frame_bytes),
        )
        .await
        .map_err(|_| PluginServiceError::TimedOut)??;
        Ok(())
    }
}

struct ServiceProcessInner {
    fence: PluginServiceFence,
    process_id: u32,
    commands: mpsc::Sender<ActorCommand>,
    limits: PluginServiceProcessLimits,
    stop_requested: AtomicBool,
    completion: watch::Receiver<Option<PluginServiceResult<()>>>,
}

pub struct NodePluginServiceProcess {
    inner: Arc<ServiceProcessInner>,
}

impl std::fmt::Debug for NodePluginServiceProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodePluginServiceProcess")
            .field("fence", &self.inner.fence)
            .field("process_id", &self.inner.process_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PluginServiceProcess for NodePluginServiceProcess {
    fn fence(&self) -> &PluginServiceFence {
        &self.inner.fence
    }

    fn process_id(&self) -> u32 {
        self.inner.process_id
    }

    async fn invoke(
        &self,
        invocation: PluginServiceInvocation,
        cancellation: PluginServiceCancellation,
    ) -> PluginServiceResult<JsonValue> {
        if cancellation.is_canceled() {
            return Err(PluginServiceError::Canceled);
        }
        let (reply, response) = oneshot::channel();
        self.inner
            .commands
            .try_send(ActorCommand::Invoke {
                invocation,
                cancellation: cancellation.clone(),
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PluginServiceError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => {
                    PluginServiceError::Crashed("service command channel closed".into())
                }
            })?;
        let mut cancel_on_drop = CancelOnDrop {
            cancellation,
            armed: true,
        };
        let result = response.await.unwrap_or_else(|_| {
            Err(PluginServiceError::Crashed(
                "service response channel closed".into(),
            ))
        });
        cancel_on_drop.armed = false;
        result
    }

    async fn stop(&self) -> PluginServiceResult<()> {
        if self
            .inner
            .stop_requested
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            match self.inner.commands.try_send(ActorCommand::Stop) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(command)) => {
                    self.inner.commands.send(command).await.map_err(|_| {
                        PluginServiceError::Crashed("service command channel closed".into())
                    })?;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
        let mut completion = self.inner.completion.clone();
        tokio::time::timeout(
            self.inner.limits.shutdown_timeout + self.inner.limits.shutdown_timeout,
            async {
                loop {
                    if let Some(result) = completion.borrow_and_update().clone() {
                        return result;
                    }
                    completion.changed().await.map_err(|_| {
                        PluginServiceError::Crashed("service completion channel closed".into())
                    })?;
                }
            },
        )
        .await
        .map_err(|_| PluginServiceError::TimedOut)?
    }

    fn terminal_result(&self) -> Option<PluginServiceResult<()>> {
        self.inner.completion.borrow().clone()
    }
}

struct CancelOnDrop {
    cancellation: PluginServiceCancellation,
    armed: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}

fn spawn_host_request(
    context: Arc<ServiceContext>,
    frame: HostRequestFrame,
    call_chain: Vec<String>,
    cancellation: PluginServiceCancellation,
    sender: mpsc::Sender<ActorCommand>,
    timeout: Duration,
) {
    tokio::spawn(async move {
        let request_id = frame.request_id;
        let operation = frame.operation;
        let payload = frame.payload;
        let result = match tokio::time::timeout(
            timeout,
            handle_host_request(
                context,
                &operation,
                payload,
                call_chain,
                cancellation.clone(),
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                cancellation.cancel();
                Err(PluginServicePortError::new("host_request_timeout"))
            }
        };
        let _ = sender
            .send(ActorCommand::HostCompleted { request_id, result })
            .await;
    });
}

async fn handle_host_request(
    context: Arc<ServiceContext>,
    operation: &str,
    payload: JsonValue,
    call_chain: Vec<String>,
    cancellation: PluginServiceCancellation,
) -> Result<JsonValue, PluginServicePortError> {
    if cancellation.is_canceled() {
        return Err(PluginServicePortError::new("service_call_canceled"));
    }
    match operation {
        "kv.get" | "kv.read" | "kv.set" | "kv.delete" | "kv.compareAndSwap"
        | "db.query" | "db.execute" | "db.batch" | "files.read" | "files.write"
        | "files.list" | "files.delete" => {
            let storage = context.storage.clone();
            let operation = operation.to_owned();
            let result = tokio::task::spawn_blocking(move || {
                handle_storage_request(&storage, &operation, payload)
            })
            .await
            .map_err(|_| PluginServicePortError::new("storage_failed"))?
            .map_err(|_| PluginServicePortError::new("storage_failed"))?;
            if cancellation.is_canceled() {
                Err(PluginServicePortError::new("service_call_canceled"))
            } else {
                Ok(result)
            }
        }
        "cache.get" => {
            let key = required_string(&payload, "key")?;
            Ok(json!({"value": context.cache.get(key).map_err(|_| PluginServicePortError::new("cache_failed"))?}))
        }
        "cache.set" => {
            let key = required_string(&payload, "key")?;
            let value = payload.get("value").cloned().unwrap_or(JsonValue::Null);
            let ttl = match payload.get("ttlMs") {
                None | Some(JsonValue::Null) => None,
                Some(value) => {
                    let millis = value.as_u64().ok_or_else(|| PluginServicePortError::new("cache_request_invalid"))?;
                    Some(Duration::from_millis(millis))
                }
            };
            context.cache.set(key, value, ttl).map_err(|_| PluginServicePortError::new("cache_failed"))?;
            Ok(JsonValue::Null)
        }
        "cache.delete" => {
            let key = required_string(&payload, "key")?;
            Ok(json!({"deleted": context.cache.delete(key).map_err(|_| PluginServicePortError::new("cache_failed"))?}))
        }
        "secrets.get" => {
            let slot = required_string(&payload, "slot")?;
            if !context.grants.secret_slots.contains(slot) {
                return Err(PluginServicePortError::new("secret_denied"));
            }
            let credential_id = context
                .credential_bindings
                .get(slot)
                .ok_or_else(|| PluginServicePortError::new("secret_denied"))?;
            let secret = context
                .ports
                .secrets
                .get(&context.plugin_id, slot, credential_id, context.preview)
                .await?;
            Ok(json!({"value": secret.as_ref().map(PluginSecret::expose)}))
        }
        "host.invoke" => {
            let capability = required_string(&payload, "capability")?;
            if !context.grants.host_capabilities.contains(capability) {
                return Err(PluginServicePortError::new("host_capability_denied"));
            }
            let input = payload.get("input").cloned().unwrap_or(JsonValue::Null);
            context
                .ports
                .host
                .invoke(
                    &context.plugin_id,
                    capability,
                    input,
                    context.preview,
                    cancellation,
                )
                .await
        }
        "actions.invoke" => {
            let action = required_string(&payload, "action")?;
            if !context.grants.allow_action_invoke {
                return Err(PluginServicePortError::new("action_denied"));
            }
            let input = payload.get("input").cloned().unwrap_or(JsonValue::Null);
            context
                .ports
                .actions
                .invoke(
                    &context.plugin_id,
                    action,
                    input,
                    call_chain,
                    context.preview,
                    cancellation,
                )
                .await
        }
        _ => Err(PluginServicePortError::new("host_operation_unsupported")),
    }
}

fn handle_storage_request(
    storage: &PluginStorage,
    operation: &str,
    payload: JsonValue,
) -> Result<JsonValue, ()> {
    match operation {
        "kv.get" | "kv.read" => {
            let key = required_string_untyped(&payload, "key")?;
            let value = storage.kv_get(key).map_err(|_| ())?;
            Ok(json!({"value": value.value, "revision": value.revision}))
        }
        "kv.set" => {
            let key = required_string_untyped(&payload, "key")?;
            let value = payload.get("value").cloned().unwrap_or(JsonValue::Null);
            let revision = storage.kv_set(key, &value).map_err(|_| ())?;
            Ok(json!({"revision": revision}))
        }
        "kv.delete" => {
            let key = required_string_untyped(&payload, "key")?;
            Ok(json!({"deleted": storage.kv_delete(key).map_err(|_| ())?}))
        }
        "kv.compareAndSwap" => {
            let key = required_string_untyped(&payload, "key")?;
            let expected = payload.get("expectedRevision").and_then(JsonValue::as_u64);
            let value = payload
                .get("hasValue")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
                .then(|| payload.get("value").cloned().unwrap_or(JsonValue::Null));
            let KvCompareAndSwap { applied, revision } = storage
                .kv_compare_and_swap(key, expected, value.as_ref())
                .map_err(|_| ())?;
            Ok(json!({"applied": applied, "revision": revision}))
        }
        "db.query" => {
            let statement = parse_statement(&payload)?;
            query_to_json(storage.db_query(&statement).map_err(|_| ())?)
        }
        "db.execute" => {
            let statement = parse_statement(&payload)?;
            let result = storage.db_execute(&statement).map_err(|_| ())?;
            Ok(json!({"affectedRows": result.affected_rows}))
        }
        "db.batch" => {
            let statements = payload
                .get("statements")
                .and_then(JsonValue::as_array)
                .ok_or(())?
                .iter()
                .map(parse_statement)
                .collect::<Result<Vec<_>, _>>()?;
            let results = storage.db_batch(&statements).map_err(|_| ())?;
            Ok(JsonValue::Array(
                results
                    .into_iter()
                    .map(|result| json!({"affectedRows": result.affected_rows}))
                    .collect(),
            ))
        }
        "files.read" => {
            let path = required_string_untyped(&payload, "path")?;
            let bytes = storage.file_read(path).map_err(|_| ())?;
            Ok(json!({"bytesHex": hex::encode(bytes)}))
        }
        "files.write" => {
            let path = required_string_untyped(&payload, "path")?;
            let bytes = payload
                .get("bytesHex")
                .and_then(JsonValue::as_str)
                .ok_or(())?;
            let bytes = hex::decode(bytes).map_err(|_| ())?;
            storage.file_write(path, &bytes).map_err(|_| ())?;
            Ok(JsonValue::Null)
        }
        "files.list" => {
            let path = payload.get("path").and_then(JsonValue::as_str);
            let entries = storage.file_list(path).map_err(|_| ())?;
            Ok(JsonValue::Array(entries.into_iter().map(file_entry_json).collect()))
        }
        "files.delete" => {
            let path = required_string_untyped(&payload, "path")?;
            Ok(json!({"deleted": storage.file_delete(path).map_err(|_| ())?}))
        }
        _ => Err(()),
    }
}

fn required_string<'a>(
    value: &'a JsonValue,
    key: &str,
) -> Result<&'a str, PluginServicePortError> {
    required_string_untyped(value, key)
        .map_err(|_| PluginServicePortError::new("host_request_invalid"))
}

fn required_string_untyped<'a>(value: &'a JsonValue, key: &str) -> Result<&'a str, ()> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(())
}

fn parse_statement(value: &JsonValue) -> Result<PluginSqlStatement, ()> {
    let sql = required_string_untyped(value, "sql")?;
    let parameters = value
        .get("parameters")
        .and_then(JsonValue::as_array)
        .ok_or(())?
        .iter()
        .map(sql_value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PluginSqlStatement::new(sql, parameters))
}

fn sql_value_from_json(value: &JsonValue) -> Result<PluginSqlValue, ()> {
    match value {
        JsonValue::Null => Ok(PluginSqlValue::Null),
        JsonValue::Bool(value) => Ok(PluginSqlValue::Integer(i64::from(*value))),
        JsonValue::Number(value) => value
            .as_i64()
            .map(PluginSqlValue::Integer)
            .or_else(|| value.as_f64().map(PluginSqlValue::Real))
            .ok_or(()),
        JsonValue::String(value) => Ok(PluginSqlValue::Text(value.clone())),
        JsonValue::Object(value) if value.len() == 1 => value
            .get("bytesHex")
            .and_then(JsonValue::as_str)
            .ok_or(())
            .and_then(|value| hex::decode(value).map_err(|_| ()))
            .map(PluginSqlValue::Blob),
        _ => Err(()),
    }
}

fn sql_value_json(value: PluginSqlValue) -> JsonValue {
    match value {
        PluginSqlValue::Null => JsonValue::Null,
        PluginSqlValue::Integer(value) => json!(value),
        PluginSqlValue::Real(value) if value.is_finite() => json!(value),
        PluginSqlValue::Real(_) => JsonValue::Null,
        PluginSqlValue::Text(value) => JsonValue::String(value),
        PluginSqlValue::Blob(value) => json!({"bytesHex": hex::encode(value)}),
    }
}

fn query_to_json(result: PluginQueryResult) -> Result<JsonValue, ()> {
    Ok(json!({
        "columns": result.columns,
        "rows": result.rows.into_iter().map(|row| {
            JsonValue::Array(row.into_iter().map(sql_value_json).collect())
        }).collect::<Vec<_>>(),
    }))
}

fn file_entry_json(entry: PluginFileEntry) -> JsonValue {
    json!({
        "path": entry.path,
        "isDirectory": entry.is_directory,
        "sizeBytes": entry.size_bytes,
    })
}

async fn read_until_hello<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    stdin: &mut ChildStdin,
    fence: &PluginServiceFence,
    process_id: u32,
    context: &Arc<ServiceContext>,
    limits: &PluginServiceProcessLimits,
) -> PluginServiceResult<ServiceHello> {
    loop {
        let value = read_json_line::<JsonValue>(reader, limits.max_frame_bytes).await?;
        match value.get("kind").and_then(JsonValue::as_str) {
            Some("hello") => {
                return serde_json::from_value(value)
                    .map_err(|_| PluginServiceError::Protocol("invalid Hello".into()));
            }
            Some("host_request") => {
                let request: HostRequestFrame = serde_json::from_value(value)
                    .map_err(|_| PluginServiceError::Protocol("invalid startup Host request".into()))?;
                if request.parent_request_id.is_some()
                    || request.protocol_version != SERVICE_PROTOCOL_VERSION
                    || request.process_generation != fence.process_generation
                {
                    return Err(PluginServiceError::Protocol(
                        "startup Host request fence mismatch".into(),
                    ));
                }
                let request_id = request.request_id.clone();
                let result = tokio::time::timeout(
                    limits.request_timeout,
                    handle_host_request(
                        Arc::clone(context),
                        &request.operation,
                        request.payload,
                        Vec::new(),
                        PluginServiceCancellation::default(),
                    ),
                )
                .await
                .map_err(|_| PluginServiceError::TimedOut)?;
                let (outcome, value, error) = match result {
                    Ok(value) => ("success", Some(value), None),
                    Err(error) => ("failure", None, Some(error)),
                };
                let error_code = error.as_ref().map(PluginServicePortError::code);
                let response = HostResponseFrame {
                    kind: "host_response", protocol_version: SERVICE_PROTOCOL_VERSION,
                    process_generation: fence.process_generation, request_id: &request_id,
                    outcome, value: value.as_ref(), error: error_code.map(|code| HostWireError { code }),
                };
                write_json_line(stdin, &response, limits.max_frame_bytes).await?;
            }
            _ => {
                let _ = process_id;
                return Err(PluginServiceError::Protocol(
                    "service emitted an unsupported startup frame".into(),
                ));
            }
        }
    }
}

fn validate_hello(
    hello: &ServiceHello,
    fence: &PluginServiceFence,
    process_id: u32,
) -> PluginServiceResult<()> {
    if hello.kind != "hello"
        || hello.protocol_version != SERVICE_PROTOCOL_VERSION
        || hello.process_generation != fence.process_generation
        || hello.process_id != process_id
        || hello.plugin_id != fence.plugin_id.as_ref()
        || hello.artifact_digest != fence.artifact_digest.as_ref()
        || hello.data_generation != fence.data_generation.as_str()
    {
        return Err(PluginServiceError::Protocol("Hello fence mismatch".into()));
    }
    Ok(())
}

async fn read_frames(
    mut reader: BufReader<ChildStdout>,
    sender: mpsc::Sender<ReaderEvent>,
    max_frame_bytes: usize,
) {
    loop {
        let value = read_json_line::<JsonValue>(&mut reader, max_frame_bytes).await;
        let event = match value {
            Ok(value) => match value.get("kind").and_then(JsonValue::as_str) {
                Some("response") => serde_json::from_value(value)
                    .map(ResponseFrame::into)
                    .map(ReaderEvent::Frame)
                    .unwrap_or_else(|_| ReaderEvent::Failed("invalid service response".into())),
                Some("host_request") => serde_json::from_value(value)
                    .map(HostRequestFrame::into)
                    .map(ReaderEvent::Frame)
                    .unwrap_or_else(|_| ReaderEvent::Failed("invalid Host request".into())),
                _ => ReaderEvent::Failed("unsupported service frame".into()),
            },
            Err(PluginServiceError::Protocol(reason)) if reason == "service IPC reached EOF" => {
                ReaderEvent::Eof
            }
            Err(error) => ReaderEvent::Failed(error.to_string()),
        };
        let terminal = matches!(event, ReaderEvent::Eof | ReaderEvent::Failed(_));
        if sender.send(event).await.is_err() || terminal {
            return;
        }
    }
}

impl From<ResponseFrame> for InboundFrame {
    fn from(value: ResponseFrame) -> Self {
        Self::Response(value)
    }
}

impl From<HostRequestFrame> for InboundFrame {
    fn from(value: HostRequestFrame) -> Self {
        Self::Host(value)
    }
}

async fn read_json_line<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncBufRead + Unpin),
    max_frame_bytes: usize,
) -> PluginServiceResult<T> {
    let mut bytes = Vec::new();
    let read = reader
        .take(max_frame_bytes as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(|_| PluginServiceError::Protocol("service IPC read failed".into()))?;
    if read == 0 {
        return Err(PluginServiceError::Protocol("service IPC reached EOF".into()));
    }
    if bytes.len() > max_frame_bytes || bytes.last() != Some(&b'\n') {
        return Err(PluginServiceError::Protocol(
            "service IPC frame exceeded its budget".into(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| PluginServiceError::Protocol("service IPC frame is invalid".into()))
}

async fn write_json_line<T: Serialize>(
    writer: &mut ChildStdin,
    value: &T,
    max_frame_bytes: usize,
) -> PluginServiceResult<()> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| PluginServiceError::Protocol("service IPC encoding failed".into()))?;
    bytes.push(b'\n');
    if bytes.len() > max_frame_bytes {
        return Err(PluginServiceError::Protocol(
            "service IPC frame exceeded its budget".into(),
        ));
    }
    writer
        .write_all(&bytes)
        .await
        .map_err(|_| PluginServiceError::Protocol("service IPC write failed".into()))?;
    writer
        .flush()
        .await
        .map_err(|_| PluginServiceError::Protocol("service IPC flush failed".into()))
}

async fn drain_stderr(mut stderr: ChildStderr) {
    let mut buffer = [0u8; 8 * 1024];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

fn validate_module_path(path: &Path) -> PluginServiceResult<PathBuf> {
    if !path.is_absolute()
        || path.file_name().and_then(|value| value.to_str()) != Some(SERVICE_ENTRYPOINT)
        || path.parent().and_then(Path::file_name).and_then(|value| value.to_str())
            != Some("service")
    {
        return Err(PluginServiceError::InvalidConfiguration(
            "service module must be an absolute service/main.mjs path".into(),
        ));
    }
    require_regular_file(path, MAX_MODULE_BYTES)?;
    fs::canonicalize(path)
        .map_err(|_| PluginServiceError::InvalidConfiguration("service module unavailable".into()))
}

fn validate_migration_module_path(
    path: &Path,
    data_root: &PluginDataRootHandle,
) -> PluginServiceResult<PathBuf> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_prefix(".migration-"))
        .and_then(|value| value.strip_suffix(".mjs"));
    let valid_name = name
        .and_then(|value| Uuid::parse_str(value).ok().map(|id| (value, id)))
        .is_some_and(|(value, id)| id.get_version_num() == 7 && id.to_string() == value);
    if !path.is_absolute()
        || !matches!(data_root.kind(), DataRootKind::Staging | DataRootKind::Preview)
        || path.parent() != Some(data_root.path())
        || !valid_name
    {
        return Err(PluginServiceError::InvalidConfiguration(
            "migration module must be an exact DataRoot .migration-<uuidv7>.mjs child".into(),
        ));
    }
    require_regular_file(path, MAX_MODULE_BYTES)?;
    let canonical = fs::canonicalize(path).map_err(|_| {
        PluginServiceError::InvalidConfiguration("migration module unavailable".into())
    })?;
    if canonical.parent() != Some(data_root.path()) {
        return Err(PluginServiceError::InvalidConfiguration(
            "migration module escaped its DataRoot".into(),
        ));
    }
    Ok(canonical)
}

fn require_regular_file(path: &Path, maximum: u64) -> PluginServiceResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| PluginServiceError::InvalidConfiguration("required file unavailable".into()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(PluginServiceError::InvalidConfiguration(
            "required file must be a bounded regular non-link file".into(),
        ));
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

impl Drop for ServiceProcessInner {
    fn drop(&mut self) {
        if !self.stop_requested.swap(true, Ordering::AcqRel) {
            let _ = self.commands.try_send(ActorCommand::Stop);
        }
    }
}

impl Clone for NodePluginServiceProcess {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod module_path_tests {
    use super::*;
    use crate::PluginDataRootManager;

    #[test]
    fn migration_wrapper_is_an_exact_regular_staging_child() {
        let temp = tempfile::tempdir().unwrap();
        let roots = PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap();
        let staged = roots
            .stage_empty(
                PluginId::from(Uuid::now_v7().to_string()),
                DataGeneration::new(Uuid::now_v7().to_string()).unwrap(),
            )
            .unwrap();
        let name = format!(".migration-{}.mjs", Uuid::now_v7());
        let wrapper = staged.path().join(&name);
        fs::write(&wrapper, b"export async function migrate() {}\n").unwrap();
        assert_eq!(
            validate_migration_module_path(&wrapper, &staged).unwrap(),
            fs::canonicalize(&wrapper).unwrap()
        );

        let outside_parent = temp.path().join("outside");
        fs::create_dir(&outside_parent).unwrap();
        let outside = outside_parent.join(&name);
        fs::write(&outside, b"export async function migrate() {}\n").unwrap();
        assert!(validate_migration_module_path(&outside, &staged).is_err());

        let wrong_name = staged.path().join("migration-main.mjs");
        fs::write(&wrong_name, b"export async function migrate() {}\n").unwrap();
        assert!(validate_migration_module_path(&wrong_name, &staged).is_err());

        fs::remove_file(&wrapper).unwrap();
        fs::create_dir(&wrapper).unwrap();
        assert!(validate_migration_module_path(&wrapper, &staged).is_err());
    }
}
