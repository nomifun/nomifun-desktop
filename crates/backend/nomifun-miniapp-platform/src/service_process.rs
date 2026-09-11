//! Production-oriented MiniApp Service process adapter.
//!
//! This module is intentionally kept separate from the in-memory Service Host
//! coordinator. It owns one Node process, speaks the dedicated MiniApp Service
//! NDJSON protocol, and turns process-tree failure into a generation-scoped
//! `MiniAppServiceProcessError::Crashed`.
//!
//! The adapter is exported by the crate, while production composition supplies
//! the committed Runtime and exact Release module resolver.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_contracts::{
    DigestHex, MiniAppReleaseRef, MiniAppServiceRuntimeFingerprint,
    MINIAPP_SERVICE_HOST_PROTOCOL_VERSION, ResolvedMiniAppServiceSpec, StrictJsonValue,
    digest_bytes,
};
use nomifun_js_runtime::{
    CommittedRuntimeProvider, JavaScriptWorkKind, RuntimeUseLease,
};
use serde::{Deserialize, Serialize};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader,
};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    MiniAppCallCancellation, MiniAppPlatformError, MiniAppPlatformResult,
    MiniAppServiceGenerationFence, MiniAppServiceInvocation, MiniAppServiceLaunch,
    MiniAppServiceProcess, MiniAppServiceProcessError, MiniAppServiceProcessFactory,
    MiniAppServiceStoragePort, MiniAppServiceStorageRequest,
};

const SERVICE_HOST_ROLE: &str = "miniapp_service";
const SERVICE_PROTOCOL_VERSION: &str = MINIAPP_SERVICE_HOST_PROTOCOL_VERSION;
const SERVICE_HOST_SCRIPT: &str = r#"
import { AsyncLocalStorage } from "node:async_hooks";
import readline from "node:readline";
import { pathToFileURL } from "node:url";

const bootstrap = JSON.parse(
  Buffer.from(process.argv.at(-1) ?? "", "hex").toString("utf8"),
);

if (
  bootstrap.host_role !== "miniapp_service" ||
  bootstrap.protocol_version !== "1.0.0" ||
  !Number.isSafeInteger(bootstrap.host_generation) ||
  bootstrap.host_generation <= 0 ||
  typeof bootstrap.module_path !== "string" ||
  bootstrap.module_path.length === 0
) {
  throw new Error("invalid MiniApp Service Host bootstrap");
}

const generation = bootstrap.host_generation;
const protocolVersion = bootstrap.protocol_version;
const activeRequests = new Map();
const storageRequests = new Map();
let storageRequestSequence = 0;
const invocationContext = new AsyncLocalStorage();
let shuttingDown = false;
let writeChain = Promise.resolve();
let service = null;

function writeFrame(frame) {
  const line = `${JSON.stringify(frame)}\n`;
  writeChain = writeChain.then(
    () =>
      new Promise((resolve, reject) => {
        process.stdout.write(line, (error) => {
          if (error) reject(error);
          else resolve();
        });
      }),
  );
  return writeChain;
}

function success(requestId, callId, value) {
  return {
    kind: "response",
    protocol_version: protocolVersion,
    host_generation: generation,
    request_id: requestId,
    call_id: callId,
    outcome: "success",
    value: value ?? null,
  };
}

function failure(requestId, callId, code, message, retryable = false) {
  return {
    kind: "response",
    protocol_version: protocolVersion,
    host_generation: generation,
    request_id: requestId,
    call_id: callId,
    outcome: "failure",
    error: {
      code,
      message: String(message),
      retryable,
    },
  };
}

function controlAck(requestId) {
  return {
    kind: "response",
    protocol_version: protocolVersion,
    host_generation: generation,
    request_id: requestId,
    outcome: "ack",
  };
}

function storageError(code, message) {
  const error = new Error(String(message));
  error.name = String(code || "storage_error");
  return error;
}

function requestStorage(operation, payload) {
  const requestId = `${generation}-storage-${++storageRequestSequence}`;
  const context = invocationContext.getStore();
  const parentRequestId = context?.requestId;
  const signal = context?.signal;
  return new Promise((resolve, reject) => {
    const abort = () => {
      const pending = storageRequests.get(requestId);
      if (!pending) return;
      storageRequests.delete(requestId);
      pending.reject(storageError("storage_call_canceled", "Service call was canceled"));
    };
    if (signal?.aborted) {
      abort();
      return;
    }
    signal?.addEventListener("abort", abort, { once: true });
    storageRequests.set(requestId, { resolve, reject, signal, abort });
    void writeFrame({
      kind: "storage_request",
      protocol_version: protocolVersion,
      host_generation: generation,
      request_id: requestId,
      parent_request_id: parentRequestId,
      operation,
      payload,
    }).catch((error) => {
      const pending = storageRequests.get(requestId);
      if (!pending) return;
      storageRequests.delete(requestId);
      pending.signal?.removeEventListener("abort", pending.abort);
      pending.reject(storageError("storage_ipc_failed", error));
    });
  });
}

function resolveStorageResponse(frame) {
  const pending = storageRequests.get(frame.request_id);
  if (!pending) return;
  storageRequests.delete(frame.request_id);
  pending.signal?.removeEventListener("abort", pending.abort);
  if (frame.outcome === "success") {
    pending.resolve(frame.value ?? null);
  } else {
    const error = frame.error || {};
    pending.reject(storageError(error.code, error.message));
  }
}

function createStorageContext(storage) {
  const filesDir = storage?.files_dir?.absolute_path ?? null;
  const database = storage?.private_database
    ? Object.freeze({
        query: (sql, parameters = []) =>
          requestStorage("database_query", { sql, parameters }),
        execute: (sql, parameters = []) =>
          requestStorage("database_execute", { sql, parameters }),
        batch: (statements) =>
          requestStorage("database_batch", { statements }),
      })
    : null;
  return Object.freeze({
    filesDir,
    kv: Object.freeze({
      get: (key) => requestStorage("kv", { operation: "get", key }),
      set: (key, value) =>
        requestStorage("kv", { operation: "set", key, value }),
      delete: (key) => requestStorage("kv", { operation: "delete", key }),
      compareAndSwap: (key, expectedRevision, value) =>
        requestStorage("kv", {
          operation: "compare_and_swap",
          key,
          expected_revision: expectedRevision,
          value,
        }),
    }),
    database,
  });
}

const lines = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
  terminal: false,
});

lines.on("line", (line) => {
  let frame;
  try {
    frame = JSON.parse(line);
  } catch (error) {
    throw new Error(`invalid MiniApp Service NDJSON frame: ${String(error)}`);
  }
  void dispatch(frame).catch((error) => {
    const requestId =
      typeof frame?.request_id === "string"
        ? frame.request_id
        : "invalid-request";
    const callId =
      typeof frame?.call_id === "string" ? frame.call_id : undefined;
    void writeFrame(
      failure(
        requestId,
        callId,
        "service_protocol_error",
        error?.message ?? String(error),
        false,
      ),
    );
  });
});

lines.on("close", () => {
  if (!shuttingDown) process.exitCode = 72;
});

const moduleUrl = pathToFileURL(bootstrap.module_path);
moduleUrl.searchParams.set("service_run_key", bootstrap.service_run_key);
const imported = await import(moduleUrl.href);
if (typeof imported.start !== "function") {
  throw new Error("service/main.mjs must export start(context)");
}

service = await imported.start(
  Object.freeze({
    miniappId: bootstrap.miniapp_id,
    release: structuredClone(bootstrap.release),
    activeReleaseEpoch: bootstrap.active_release_epoch,
    serviceRunKey: bootstrap.service_run_key,
    hostGeneration: generation,
    runtime: structuredClone(bootstrap.runtime),
    storage: createStorageContext(bootstrap.storage),
  }),
);
if (!service || typeof service !== "object" || typeof service.invoke !== "function") {
  throw new Error("start(context) must return { invoke(...) }");
}

await writeFrame({
  kind: "hello",
  host_role: bootstrap.host_role,
  protocol_version: protocolVersion,
  host_generation: generation,
  process_id: process.pid,
  miniapp_id: bootstrap.miniapp_id,
  release: structuredClone(bootstrap.release),
  active_release_epoch: bootstrap.active_release_epoch,
  service_run_key: bootstrap.service_run_key,
  module_digest: bootstrap.module_digest,
  runtime: structuredClone(bootstrap.runtime),
});

async function dispatch(frame) {
  if (
    !frame ||
    frame.protocol_version !== protocolVersion ||
    frame.host_generation !== generation
  ) {
    throw new Error("Service request does not bind the active generation");
  }

  if (frame.kind === "storage_response") {
    resolveStorageResponse(frame);
    return;
  }

  if (frame.kind === "control" && frame.operation === "cancel") {
    activeRequests.get(frame.target_request_id)?.abort();
    await writeFrame(controlAck(frame.request_id));
    return;
  }

  if (frame.kind === "control" && frame.operation === "shutdown") {
    shuttingDown = true;
    if (typeof service.dispose === "function") {
      await service.dispose();
    }
    await writeFrame(controlAck(frame.request_id));
    process.stdin.pause();
    return;
  }

  if (
    frame.kind !== "request" ||
    frame.operation !== "invoke" ||
    typeof frame.request_id !== "string" ||
    typeof frame.call_id !== "string" ||
    typeof frame.method !== "string"
  ) {
    throw new Error("unsupported MiniApp Service request");
  }

  const controller = new AbortController();
  activeRequests.set(frame.request_id, controller);
  try {
    const value = await invocationContext.run(
      { requestId: frame.request_id, signal: controller.signal },
      () =>
        service.invoke(
          Object.freeze({
            callId: frame.call_id,
            method: frame.method,
            payload: structuredClone(frame.payload ?? {}),
            signal: controller.signal,
          }),
        ),
    );
    await writeFrame(success(frame.request_id, frame.call_id, value));
  } catch (error) {
    await writeFrame(
      failure(
        frame.request_id,
        frame.call_id,
        error?.name === "AbortError"
          ? "service_call_canceled"
          : "service_invocation_failed",
        error?.message ?? String(error),
        error?.name === "AbortError",
      ),
    );
  } finally {
    activeRequests.delete(frame.request_id);
  }
}

process.on("uncaughtException", (error) => {
  process.stderr.write(`uncaught exception: ${String(error)}\n`);
  process.exitCode = 70;
});

process.on("unhandledRejection", (error) => {
  process.stderr.write(`unhandled rejection: ${String(error)}\n`);
  process.exitCode = 71;
});
"#;

#[derive(Clone, Debug)]
pub struct MiniAppServiceProcessLimits {
    pub hello_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_frame_bytes: usize,
    pub command_queue_capacity: usize,
    pub cancellation_poll_interval: Duration,
}

impl Default for MiniAppServiceProcessLimits {
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

impl MiniAppServiceProcessLimits {
    fn validate(&self) -> Result<(), MiniAppPlatformError> {
        if self.hello_timeout.is_zero()
            || self.request_timeout.is_zero()
            || self.shutdown_timeout.is_zero()
            || self.max_frame_bytes == 0
            || self.command_queue_capacity == 0
            || self.cancellation_poll_interval.is_zero()
        {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service process limits must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
pub trait MiniAppServiceModuleResolver: Send + Sync {
    async fn resolve_module(
        &self,
        launch: &MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<PathBuf>;
}

#[derive(Clone, Debug)]
pub struct FixedMiniAppServiceModuleResolver {
    path: PathBuf,
}

impl FixedMiniAppServiceModuleResolver {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl MiniAppServiceModuleResolver for FixedMiniAppServiceModuleResolver {
    async fn resolve_module(
        &self,
        _launch: &MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<PathBuf> {
        Ok(self.path.clone())
    }
}

pub struct NodeMiniAppServiceProcessFactory {
    node_executable: PathBuf,
    resolver: Arc<dyn MiniAppServiceModuleResolver>,
    limits: MiniAppServiceProcessLimits,
    storage: Option<Arc<dyn MiniAppServiceStoragePort>>,
}

/// Factory that acquires the committed Runtime admission lease for the full
/// lifetime of every resident Service process.
pub struct RuntimeAwareMiniAppServiceProcessFactory {
    authority: Arc<dyn CommittedRuntimeProvider>,
    resolver: Arc<dyn MiniAppServiceModuleResolver>,
    limits: MiniAppServiceProcessLimits,
    storage: Option<Arc<dyn MiniAppServiceStoragePort>>,
}

impl std::fmt::Debug for RuntimeAwareMiniAppServiceProcessFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeAwareMiniAppServiceProcessFactory")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl RuntimeAwareMiniAppServiceProcessFactory {
    pub fn new(
        authority: Arc<dyn CommittedRuntimeProvider>,
        resolver: Arc<dyn MiniAppServiceModuleResolver>,
    ) -> Self {
        Self {
            authority,
            resolver,
            limits: MiniAppServiceProcessLimits::default(),
            storage: None,
        }
    }

    pub fn with_limits(
        mut self,
        limits: MiniAppServiceProcessLimits,
    ) -> Result<Self, MiniAppPlatformError> {
        limits.validate()?;
        self.limits = limits;
        Ok(self)
    }

    pub fn with_storage(
        mut self,
        storage: Arc<dyn MiniAppServiceStoragePort>,
    ) -> Self {
        self.storage = Some(storage);
        self
    }
}

struct RuntimeLeasedMiniAppServiceProcess {
    inner: Arc<dyn MiniAppServiceProcess>,
    _lease: RuntimeUseLease,
}

#[async_trait]
impl MiniAppServiceProcess for RuntimeLeasedMiniAppServiceProcess {
    async fn invoke(
        &self,
        invocation: MiniAppServiceInvocation,
        cancellation: MiniAppCallCancellation,
    ) -> Result<StrictJsonValue, MiniAppServiceProcessError> {
        self.inner.invoke(invocation, cancellation).await
    }

    async fn stop(&self) {
        self.inner.stop().await;
    }

    fn terminal_result(&self) -> Option<Result<(), String>> {
        self.inner.terminal_result()
    }
}

#[async_trait]
impl MiniAppServiceProcessFactory for RuntimeAwareMiniAppServiceProcessFactory {
    async fn start(
        &self,
        launch: MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<Arc<dyn MiniAppServiceProcess>> {
        self.limits.validate()?;
        let lease = self
            .authority
            .acquire_use(JavaScriptWorkKind::MiniappServiceHost)
            .await
            .map_err(|error| MiniAppPlatformError::Runtime(error.to_string()))?;
        let runtime = lease.runtime();
        let expected_runtime = MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: runtime.fingerprint.runtime_installation_id.clone(),
            runtime_target: runtime.fingerprint.runtime_target.clone(),
            runtime_executable_digest: runtime.fingerprint.executable_digest.clone(),
            node_version: runtime.fingerprint.node_version.clone(),
        };
        if launch.spec.runtime != expected_runtime {
            return Err(MiniAppPlatformError::StaleServiceGeneration);
        }
        let factory = NodeMiniAppServiceProcessFactory::new(
            runtime.executable_path.clone(),
            Arc::clone(&self.resolver),
        )?
        .with_limits(self.limits.clone())?;
        let factory = match &self.storage {
            Some(storage) => factory.with_storage(Arc::clone(storage)),
            None => factory,
        };
        let process = factory.start(launch).await?;
        Ok(Arc::new(RuntimeLeasedMiniAppServiceProcess {
            inner: process,
            _lease: lease,
        }))
    }
}

impl std::fmt::Debug for NodeMiniAppServiceProcessFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeMiniAppServiceProcessFactory")
            .field("node_executable", &self.node_executable)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl NodeMiniAppServiceProcessFactory {
    pub fn new(
        node_executable: impl Into<PathBuf>,
        resolver: Arc<dyn MiniAppServiceModuleResolver>,
    ) -> Result<Self, MiniAppPlatformError> {
        let factory = Self {
            node_executable: node_executable.into(),
            resolver,
            limits: MiniAppServiceProcessLimits::default(),
            storage: None,
        };
        factory.validate_node_executable()?;
        Ok(factory)
    }

    pub fn with_limits(
        mut self,
        limits: MiniAppServiceProcessLimits,
    ) -> Result<Self, MiniAppPlatformError> {
        limits.validate()?;
        self.limits = limits;
        Ok(self)
    }

    pub fn with_storage(
        mut self,
        storage: Arc<dyn MiniAppServiceStoragePort>,
    ) -> Self {
        self.storage = Some(storage);
        self
    }

    fn validate_node_executable(&self) -> Result<(), MiniAppPlatformError> {
        if !self.node_executable.is_absolute() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service Node executable must be absolute".into(),
            ));
        }
        let metadata = std::fs::symlink_metadata(&self.node_executable).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp Service Node executable {}: {error}",
                self.node_executable.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service Node executable must be a regular non-symlink file".into(),
            ));
        }
        Ok(())
    }

    async fn verify_runtime(
        &self,
        spec: &ResolvedMiniAppServiceSpec,
    ) -> Result<(), MiniAppPlatformError> {
        let bytes = tokio::fs::read(&self.node_executable)
            .await
            .map_err(|error| {
                MiniAppPlatformError::Runtime(format!(
                    "cannot read MiniApp Service Node executable: {error}"
                ))
            })?;
        let observed = digest_bytes(&bytes);
        if observed != spec.runtime.runtime_executable_digest {
            return Err(MiniAppPlatformError::Runtime(format!(
                "MiniApp Service Node executable digest mismatch: expected {}, observed {}",
                spec.runtime.runtime_executable_digest.as_ref(),
                observed.as_ref()
            )));
        }
        Ok(())
    }

    async fn verify_module(
        &self,
        path: PathBuf,
        expected_digest: &DigestHex,
    ) -> Result<PathBuf, MiniAppPlatformError> {
        if !path.is_absolute() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service module path must be absolute".into(),
            ));
        }
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|error| {
                MiniAppPlatformError::Runtime(format!(
                    "cannot inspect MiniApp Service module {}: {error}",
                    path.display()
                ))
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service module must be a regular non-symlink file".into(),
            ));
        }
        let canonical = tokio::fs::canonicalize(&path).await.map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot canonicalize MiniApp Service module {}: {error}",
                path.display()
            ))
        })?;
        let bytes = tokio::fs::read(&canonical).await.map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot read MiniApp Service module {}: {error}",
                canonical.display()
            ))
        })?;
        let observed = digest_bytes(&bytes);
        if &observed != expected_digest {
            return Err(MiniAppPlatformError::Runtime(format!(
                "MiniApp Service module digest mismatch: expected {}, observed {}",
                expected_digest.as_ref(),
                observed.as_ref()
            )));
        }
        Ok(canonical)
    }
}

#[async_trait]
impl MiniAppServiceProcessFactory for NodeMiniAppServiceProcessFactory {
    async fn start(
        &self,
        launch: MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<Arc<dyn MiniAppServiceProcess>> {
        self.limits.validate()?;
        launch
            .spec
            .validate()
            .map_err(|error| MiniAppPlatformError::Contract(error))?;
        if launch.host_generation == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service Host generation must be positive".into(),
            ));
        }
        self.verify_runtime(&launch.spec).await?;
        let module = self
            .resolver
            .resolve_module(&launch)
            .await
            .map_err(|error| MiniAppPlatformError::Runtime(error.to_string()))?;
        let module = self
            .verify_module(module, &launch.spec.service_module_digest)
            .await?;

        let fence = MiniAppServiceGenerationFence {
            miniapp_id: launch.spec.miniapp_id.clone(),
            release: launch.spec.release.clone(),
            active_release_epoch: launch.spec.active_release_epoch,
            service_run_key: launch.spec.service_run_key.clone(),
            host_generation: launch.host_generation,
        };
        let bootstrap = ServiceBootstrap {
            host_role: SERVICE_HOST_ROLE.to_owned(),
            protocol_version: SERVICE_PROTOCOL_VERSION.to_owned(),
            host_generation: launch.host_generation,
            miniapp_id: launch.spec.miniapp_id.as_ref().to_owned(),
            release: launch.spec.release.clone(),
            active_release_epoch: launch.spec.active_release_epoch,
            service_run_key: launch.spec.service_run_key.clone(),
            module_path: module.display().to_string(),
            module_digest: launch.spec.service_module_digest.clone(),
            runtime: launch.spec.runtime.clone(),
            storage: launch.spec.storage.clone(),
        };
        let bootstrap_json = serde_json::to_vec(&bootstrap).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot encode MiniApp Service bootstrap: {error}"
            ))
        })?;
        let bootstrap_hex = hex::encode(bootstrap_json);
        let working_directory = service_process_working_directory(
            &module,
            &self.node_executable,
        )?;

        let mut builder = ChildProcessBuilder::new(&self.node_executable);
        builder
            .arg("--input-type=module")
            .arg("-e")
            .arg(SERVICE_HOST_SCRIPT)
            .arg(&bootstrap_hex)
            .current_dir(working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = builder
            .spawn_managed()
            .map_err(|error| MiniAppPlatformError::Runtime(error.to_string()))?;
        let process_id = process.id().ok_or_else(|| {
            MiniAppPlatformError::Runtime(
                "MiniApp Service Node process did not expose a process id".into(),
            )
        })?;
        let mut stdin = process.stdin.take().ok_or_else(|| {
            MiniAppPlatformError::Runtime(
                "MiniApp Service Node stdin was not captured".into(),
            )
        })?;
        let stdout = process.stdout.take().ok_or_else(|| {
            MiniAppPlatformError::Runtime(
                "MiniApp Service Node stdout was not captured".into(),
            )
        })?;
        let stderr = process.stderr.take().ok_or_else(|| {
            MiniAppPlatformError::Runtime(
                "MiniApp Service Node stderr was not captured".into(),
            )
        })?;
        let mut reader = BufReader::new(stdout);
        let hello = match tokio::time::timeout(
            self.limits.hello_timeout,
            read_service_hello(
                &mut reader,
                &mut stdin,
                &launch.spec,
                launch.host_generation,
                self.storage.clone(),
                &self.limits,
            ),
        )
        .await
        {
            Ok(Ok(hello)) => hello,
            Ok(Err(error)) => {
                let _ = process.shutdown().await;
                return Err(MiniAppPlatformError::Runtime(format!(
                    "MiniApp Service Hello rejected: {error}"
                )));
            }
            Err(_) => {
                let _ = process.shutdown().await;
                return Err(MiniAppPlatformError::Runtime(
                    "MiniApp Service Hello timed out".into(),
                ));
            }
        };
        validate_hello(&hello, &bootstrap, process_id, &fence)?;

        let (reader_sender, reader_events) =
            mpsc::channel(self.limits.command_queue_capacity);
        let reader_task = tokio::spawn(read_service_frames(
            reader,
            reader_sender,
            self.limits.max_frame_bytes,
        ));
        let stderr_task = tokio::spawn(drain_stderr(stderr));
        let (command_sender, commands) =
            mpsc::channel(self.limits.command_queue_capacity);
        let (completion, completion_receiver) = tokio::sync::watch::channel(None);
        let actor = ServiceProcessActor {
            process,
            stdin,
            reader_events,
            commands,
            command_sender: command_sender.clone(),
            pending: BTreeMap::new(),
            call_to_request: BTreeMap::new(),
            retired_requests: BTreeMap::new(),
            storage_pending: BTreeMap::new(),
            reader_task,
            stderr_task,
            fence: fence.clone(),
            storage_descriptor: launch.spec.storage.clone(),
            storage: self.storage.clone(),
            limits: self.limits.clone(),
            accepting: true,
            completion,
        };
        tokio::spawn(actor.run());

        Ok(Arc::new(NodeMiniAppServiceProcess {
            inner: Arc::new(ServiceProcessInner {
                fence,
                commands: command_sender,
                limits: self.limits.clone(),
                stopped: AtomicBool::new(false),
                completion: completion_receiver,
            }),
        }))
    }
}

fn service_process_working_directory(
    module: &std::path::Path,
    _node_executable: &std::path::Path,
) -> MiniAppPlatformResult<PathBuf> {
    let module_directory = module.parent().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp Service module has no parent directory".into(),
        )
    })?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        // CreateProcessW does not accept an extended-length `lpCurrentDirectory`.
        // The module itself is imported by its verified absolute file URL, so a
        // short, stable cwd does not weaken module identity or relative imports.
        if module_directory.as_os_str().encode_wide().count() >= 248 {
            return _node_executable
                .parent()
                .map(PathBuf::from)
                .ok_or_else(|| {
                    MiniAppPlatformError::InvalidState(
                        "MiniApp Service Node executable has no parent directory".into(),
                    )
                });
        }
    }
    Ok(module_directory.to_path_buf())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceBootstrap {
    host_role: String,
    protocol_version: String,
    host_generation: u64,
    miniapp_id: String,
    release: MiniAppReleaseRef,
    active_release_epoch: u64,
    service_run_key: DigestHex,
    module_path: String,
    module_digest: DigestHex,
    runtime: MiniAppServiceRuntimeFingerprint,
    storage: nomifun_agent_contracts::MiniAppServiceStorageDescriptor,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceHello {
    kind: String,
    host_role: String,
    protocol_version: String,
    host_generation: u64,
    process_id: u32,
    miniapp_id: String,
    release: MiniAppReleaseRef,
    active_release_epoch: u64,
    service_run_key: DigestHex,
    module_digest: DigestHex,
    runtime: MiniAppServiceRuntimeFingerprint,
}

fn validate_hello(
    hello: &ServiceHello,
    bootstrap: &ServiceBootstrap,
    process_id: u32,
    fence: &MiniAppServiceGenerationFence,
) -> Result<(), MiniAppPlatformError> {
    if hello.kind != "hello"
        || hello.host_role != SERVICE_HOST_ROLE
        || hello.protocol_version != SERVICE_PROTOCOL_VERSION
        || hello.host_generation != fence.host_generation
        || hello.process_id != process_id
        || hello.miniapp_id != fence.miniapp_id.as_ref()
        || hello.release != fence.release
        || hello.active_release_epoch != fence.active_release_epoch
        || hello.service_run_key != fence.service_run_key
        || hello.module_digest != bootstrap.module_digest
        || hello.runtime != bootstrap.runtime
    {
        return Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Hello does not bind the exact role, generation, release, module, and runtime"
                .into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceResponseFrame {
    kind: String,
    protocol_version: String,
    host_generation: u64,
    request_id: String,
    #[serde(default)]
    call_id: Option<String>,
    outcome: String,
    #[serde(default)]
    value: Option<StrictJsonValue>,
    #[serde(default)]
    error: Option<ServiceWireError>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceStorageRequestFrame {
    kind: String,
    protocol_version: String,
    host_generation: u64,
    request_id: String,
    #[serde(default)]
    parent_request_id: Option<String>,
    operation: String,
    payload: StrictJsonValue,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ServiceStorageResponseFrame {
    kind: &'static str,
    protocol_version: &'static str,
    host_generation: u64,
    request_id: String,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<StrictJsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ServiceStorageWireError>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ServiceStorageWireError {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ServiceInboundFrame {
    Storage(ServiceStorageRequestFrame),
    Response(ServiceResponseFrame),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceWireError {
    code: String,
    message: String,
    #[allow(dead_code)]
    retryable: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct InvokeFrame<'a> {
    kind: &'static str,
    protocol_version: &'static str,
    host_generation: u64,
    request_id: &'a str,
    call_id: &'a str,
    operation: &'static str,
    method: &'a str,
    payload: &'a StrictJsonValue,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CancelFrame<'a> {
    kind: &'static str,
    protocol_version: &'static str,
    host_generation: u64,
    request_id: &'a str,
    operation: &'static str,
    target_request_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ShutdownFrame<'a> {
    kind: &'static str,
    protocol_version: &'static str,
    host_generation: u64,
    request_id: &'a str,
    operation: &'static str,
}

enum ReaderEvent {
    Frame(ServiceInboundFrame),
    Eof,
    Failed(String),
}

enum ActorCommand {
    Invoke {
        invocation: MiniAppServiceInvocation,
        cancellation: MiniAppCallCancellation,
        reply: oneshot::Sender<Result<StrictJsonValue, MiniAppServiceProcessError>>,
    },
    CancelCall {
        call_id: String,
    },
    StorageCompleted {
        request_id: String,
        result: Result<StrictJsonValue, String>,
    },
    Stop,
}

struct PendingRequest {
    call_id: Option<String>,
    cancellation: Option<MiniAppCallCancellation>,
    deadline: Instant,
    reply: PendingReply,
}

struct PendingStorageRequest {
    cancellation: MiniAppCallCancellation,
    deadline: Instant,
}

enum PendingReply {
    Invoke(oneshot::Sender<Result<StrictJsonValue, MiniAppServiceProcessError>>),
    Cancel,
    Stop,
}

enum ActorExit {
    Failed(String),
    Stopped,
}

struct ServiceProcessActor {
    process: ManagedChildProcess,
    stdin: ChildStdin,
    reader_events: mpsc::Receiver<ReaderEvent>,
    commands: mpsc::Receiver<ActorCommand>,
    command_sender: mpsc::Sender<ActorCommand>,
    pending: BTreeMap<String, PendingRequest>,
    call_to_request: BTreeMap<String, String>,
    retired_requests: BTreeMap<String, Instant>,
    storage_pending: BTreeMap<String, PendingStorageRequest>,
    reader_task: JoinHandle<()>,
    stderr_task: JoinHandle<StderrSummary>,
    fence: MiniAppServiceGenerationFence,
    storage_descriptor: nomifun_agent_contracts::MiniAppServiceStorageDescriptor,
    storage: Option<Arc<dyn MiniAppServiceStoragePort>>,
    limits: MiniAppServiceProcessLimits,
    accepting: bool,
    completion: tokio::sync::watch::Sender<Option<Result<(), String>>>,
}

impl ServiceProcessActor {
    async fn run(mut self) {
        let exit = self.run_loop().await;
        self.accepting = false;
        let failure_reason = match &exit {
            ActorExit::Failed(reason) => Some(reason.clone()),
            ActorExit::Stopped => None,
        };
        match &exit {
            ActorExit::Failed(reason) => {
                self.fail_pending(MiniAppServiceProcessError::Crashed(reason.clone()));
            }
            ActorExit::Stopped => {
                self.fail_pending(MiniAppServiceProcessError::Crashed(
                    "MiniApp Service stopped".into(),
                ));
            }
        }
        let cleanup = match tokio::time::timeout(
            self.limits.shutdown_timeout,
            self.process.shutdown(),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("MiniApp Service process-tree cleanup timed out".into()),
        };
        self.reader_task.abort();
        let _ = self.reader_task.await;
        let _ = self.stderr_task.await;
        let result = match (failure_reason, cleanup) {
            (None, Ok(())) => Ok(()),
            (Some(reason), Ok(())) => Err(reason),
            (None, Err(error)) => Err(error),
            (Some(reason), Err(error)) => Err(format!("{reason}; cleanup failed: {error}")),
        };
        self.completion.send_replace(Some(result));
    }

    async fn run_loop(&mut self) -> ActorExit {
        let tick_period = self
            .limits
            .request_timeout
            .min(Duration::from_millis(50));
        let mut watchdog = tokio::time::interval(tick_period);
        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        return ActorExit::Failed("MiniApp Service command channel closed".into());
                    };
                    if let Some(exit) = self.handle_command(command).await {
                        return exit;
                    }
                }
                event = self.reader_events.recv() => {
                    match event {
                        Some(ReaderEvent::Frame(ServiceInboundFrame::Response(frame))) => {
                            if let Some(exit) = self.handle_frame(frame) {
                                return exit;
                            }
                        }
                        Some(ReaderEvent::Frame(ServiceInboundFrame::Storage(frame))) => {
                            if let Some(exit) = self.handle_storage_request(frame).await {
                                return exit;
                            }
                        }
                        Some(ReaderEvent::Eof) | None => {
                            return ActorExit::Failed("MiniApp Service IPC reached EOF".into());
                        }
                        Some(ReaderEvent::Failed(reason)) => {
                            return ActorExit::Failed(reason);
                        }
                    }
                }
                _ = watchdog.tick() => {
                    if let Ok(Some(status)) = self.process.try_wait() {
                        return ActorExit::Failed(format!(
                            "MiniApp Service process exited with {status}"
                        ));
                    }
                    let now = Instant::now();
                    if self.pending.values().any(|pending| pending.deadline <= now)
                        || self.retired_requests.values().any(|deadline| *deadline <= now)
                    {
                        return ActorExit::Failed("MiniApp Service request watchdog timed out".into());
                    }
                    if let Some(exit) = self.expire_storage_requests(now).await {
                        return exit;
                    }
                }
            }
        }
    }

    async fn handle_command(&mut self, command: ActorCommand) -> Option<ActorExit> {
        match command {
            ActorCommand::Invoke {
                invocation,
                cancellation,
                reply,
            } => {
                if !self.accepting {
                    let _ = reply.send(Err(MiniAppServiceProcessError::Crashed(
                        "MiniApp Service is stopping".into(),
                    )));
                    return None;
                }
                if invocation.fence != self.fence {
                    let _ = reply.send(Err(MiniAppServiceProcessError::Rejected(
                        "MiniApp Service generation fence mismatch".into(),
                    )));
                    return None;
                }
                let call_id = invocation.call_id.as_ref().to_owned();
                if cancellation.is_canceled() {
                    let _ = reply.send(Err(MiniAppServiceProcessError::Rejected(
                        "MiniApp Service call canceled".into(),
                    )));
                    return None;
                }
                if self.call_to_request.contains_key(&call_id) {
                    let _ = reply.send(Err(MiniAppServiceProcessError::Rejected(
                        "MiniApp Service call is already in flight".into(),
                    )));
                    return None;
                }
                let request_id = Uuid::now_v7().to_string();
                let frame = InvokeFrame {
                    kind: "request",
                    protocol_version: SERVICE_PROTOCOL_VERSION,
                    host_generation: self.fence.host_generation,
                    request_id: &request_id,
                    call_id: &call_id,
                    operation: "invoke",
                    method: &invocation.method,
                    payload: &invocation.payload,
                };
                if let Err(error) = write_json_line(&mut self.stdin, &frame).await {
                    let _ = reply.send(Err(MiniAppServiceProcessError::Crashed(error)));
                    return Some(ActorExit::Failed(
                        "MiniApp Service IPC write failed".into(),
                    ));
                }
                self.call_to_request
                    .insert(call_id.clone(), request_id.clone());
                self.pending.insert(
                    request_id,
                    PendingRequest {
                        call_id: Some(call_id),
                        cancellation: Some(cancellation),
                        deadline: Instant::now() + self.limits.request_timeout,
                        reply: PendingReply::Invoke(reply),
                    },
                );
                None
            }
            ActorCommand::StorageCompleted { request_id, result } => {
                self.finish_storage_request(request_id, result).await
            }
            ActorCommand::CancelCall { call_id } => {
                let Some(request_id) = self.call_to_request.remove(&call_id) else {
                    return None;
                };
                if let Some(pending) = self.pending.remove(&request_id) {
                    if let PendingReply::Invoke(reply) = pending.reply {
                        let _ = reply.send(Err(MiniAppServiceProcessError::Rejected(
                            "MiniApp Service call canceled".into(),
                        )));
                    }
                }
                self.retired_requests.insert(
                    request_id.clone(),
                    Instant::now() + self.limits.request_timeout,
                );
                let cancel_id = Uuid::now_v7().to_string();
                let frame = CancelFrame {
                    kind: "control",
                    protocol_version: SERVICE_PROTOCOL_VERSION,
                    host_generation: self.fence.host_generation,
                    request_id: &cancel_id,
                    operation: "cancel",
                    target_request_id: &request_id,
                };
                if let Err(error) = write_json_line(&mut self.stdin, &frame).await {
                    return Some(ActorExit::Failed(error));
                }
                self.pending.insert(
                    cancel_id,
                    PendingRequest {
                        call_id: None,
                        cancellation: None,
                        deadline: Instant::now() + self.limits.request_timeout,
                        reply: PendingReply::Cancel,
                    },
                );
                None
            }
            ActorCommand::Stop => {
                if !self.accepting {
                    return Some(ActorExit::Stopped);
                }
                self.accepting = false;
                self.fail_pending(MiniAppServiceProcessError::Crashed(
                    "MiniApp Service is stopping".into(),
                ));
                let request_id = Uuid::now_v7().to_string();
                let frame = ShutdownFrame {
                    kind: "control",
                    protocol_version: SERVICE_PROTOCOL_VERSION,
                    host_generation: self.fence.host_generation,
                    request_id: &request_id,
                    operation: "shutdown",
                };
                if let Err(error) = write_json_line(&mut self.stdin, &frame).await {
                    return Some(ActorExit::Failed(error));
                }
                self.pending.insert(
                    request_id,
                    PendingRequest {
                        call_id: None,
                        cancellation: None,
                        deadline: Instant::now() + self.limits.shutdown_timeout,
                        reply: PendingReply::Stop,
                    },
                );
                None
            }
        }
    }

    fn handle_frame(&mut self, frame: ServiceResponseFrame) -> Option<ActorExit> {
        if frame.kind != "response"
            || frame.protocol_version != SERVICE_PROTOCOL_VERSION
            || frame.host_generation != self.fence.host_generation
        {
            return Some(ActorExit::Failed(
                "MiniApp Service response identity mismatch".into(),
            ));
        }
        let Some(pending) = self.pending.remove(&frame.request_id) else {
            if self.retired_requests.remove(&frame.request_id).is_some() {
                return None;
            }
            return Some(ActorExit::Failed(
                "MiniApp Service emitted an unknown response".into(),
            ));
        };
        if let Some(call_id) = &pending.call_id {
            if frame.call_id.as_deref() != Some(call_id.as_str()) {
                return Some(ActorExit::Failed(
                    "MiniApp Service response call identity mismatch".into(),
                ));
            }
            if self.call_to_request.get(call_id) == Some(&frame.request_id) {
                self.call_to_request.remove(call_id);
            }
        }
        match pending.reply {
            PendingReply::Invoke(reply) => {
                let result = match frame.outcome.as_str() {
                    "success" => frame.value.ok_or_else(|| {
                        MiniAppServiceProcessError::Crashed(
                            "MiniApp Service success response has no value".into(),
                        )
                    }),
                    "failure" => Err(MiniAppServiceProcessError::Rejected(
                        frame
                            .error
                            .map(|error| format!("{}: {}", error.code, error.message))
                            .unwrap_or_else(|| {
                                "MiniApp Service returned an unspecified failure".into()
                            }),
                    )),
                    _ => Err(MiniAppServiceProcessError::Crashed(
                        "MiniApp Service response outcome is invalid".into(),
                    )),
                };
                let _ = reply.send(result);
                None
            }
            PendingReply::Cancel => None,
            PendingReply::Stop => {
                if frame.outcome == "ack" {
                    Some(ActorExit::Stopped)
                } else {
                    Some(ActorExit::Failed(
                        "MiniApp Service shutdown was not acknowledged".into(),
                    ))
                }
            }
        }
    }

    async fn handle_storage_request(
        &mut self,
        frame: ServiceStorageRequestFrame,
    ) -> Option<ActorExit> {
        if frame.kind != "storage_request"
            || frame.protocol_version != SERVICE_PROTOCOL_VERSION
            || frame.host_generation != self.fence.host_generation
            || frame.request_id.trim().is_empty()
        {
            return Some(ActorExit::Failed(
                "MiniApp Service storage request identity mismatch".into(),
            ));
        }
        let request = match decode_storage_request(&frame.operation, frame.payload) {
            Ok(request) => request,
            Err(error) => {
                let response = ServiceStorageResponseFrame {
                    kind: "storage_response",
                    protocol_version: SERVICE_PROTOCOL_VERSION,
                    host_generation: self.fence.host_generation,
                    request_id: frame.request_id,
                    outcome: "failure",
                    value: None,
                    error: Some(ServiceStorageWireError {
                        code: "storage_request_invalid".into(),
                        message: error,
                    }),
                };
                if let Err(error) = write_json_line(&mut self.stdin, &response).await {
                    return Some(ActorExit::Failed(error));
                }
                return None;
            }
        };
        if self.storage_pending.contains_key(&frame.request_id) {
            return Some(ActorExit::Failed(
                "MiniApp Service storage request id was duplicated".into(),
            ));
        }
        if self.storage_pending.len() >= self.limits.command_queue_capacity {
            let response = storage_failure_response(
                self.fence.host_generation,
                frame.request_id,
                "storage_queue_full",
                "MiniApp Service storage request queue is full",
            );
            if let Err(error) = write_json_line(&mut self.stdin, &response).await {
                return Some(ActorExit::Failed(error));
            }
            return None;
        }
        let cancellation = match frame.parent_request_id.as_ref() {
            Some(request_id) => match self
                .pending
                .get(request_id)
                .and_then(|pending| pending.cancellation.clone())
            {
                Some(cancellation) => cancellation,
                None => {
                    let response = storage_failure_response(
                        self.fence.host_generation,
                        frame.request_id,
                        "storage_parent_request_stale",
                        "MiniApp Service storage parent request is stale",
                    );
                    if let Err(error) = write_json_line(&mut self.stdin, &response).await {
                        return Some(ActorExit::Failed(error));
                    }
                    return None;
                }
            },
            None => MiniAppCallCancellation::default(),
        };
        let request_id = frame.request_id;
        let storage = self.storage.clone();
        let miniapp_id = self.fence.miniapp_id.clone();
        let storage_descriptor = self.storage_descriptor.clone();
        let command_sender = self.command_sender.clone();
        let request_timeout = self.limits.request_timeout;
        self.storage_pending.insert(
            request_id.clone(),
            PendingStorageRequest {
                cancellation: cancellation.clone(),
                deadline: Instant::now() + request_timeout,
            },
        );
        tokio::spawn(async move {
            let operation_cancellation = cancellation.clone();
            let result = match storage {
                Some(storage) => match tokio::time::timeout(
                    request_timeout,
                    storage.handle_service_request(
                        &miniapp_id,
                        &storage_descriptor,
                        request,
                        operation_cancellation.clone(),
                    ),
                )
                .await
                {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(_) => {
                        operation_cancellation.cancel();
                        Err("MiniApp Service storage request timed out".into())
                    }
                },
                None => Err("MiniApp managed Service storage is not configured".into()),
            };
            let _ = command_sender
                .send(ActorCommand::StorageCompleted { request_id, result })
                .await;
        });
        None
    }

    async fn finish_storage_request(
        &mut self,
        request_id: String,
        result: Result<StrictJsonValue, String>,
    ) -> Option<ActorExit> {
        let Some(pending) = self.storage_pending.remove(&request_id) else {
            return None;
        };
        let response = match result {
            Ok(value) => ServiceStorageResponseFrame {
                kind: "storage_response",
                protocol_version: SERVICE_PROTOCOL_VERSION,
                host_generation: self.fence.host_generation,
                request_id,
                outcome: "success",
                value: Some(value),
                error: None,
            },
            Err(message) => storage_failure_response(
                self.fence.host_generation,
                request_id,
                if pending.cancellation.is_canceled() {
                    "storage_call_canceled"
                } else {
                    "storage_request_failed"
                },
                &message,
            ),
        };
        if let Err(error) = write_json_line(&mut self.stdin, &response).await {
            return Some(ActorExit::Failed(error));
        }
        None
    }

    async fn expire_storage_requests(&mut self, now: Instant) -> Option<ActorExit> {
        let expired = self
            .storage_pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in expired {
            let Some(pending) = self.storage_pending.remove(&request_id) else {
                continue;
            };
            pending.cancellation.cancel();
            let response = storage_failure_response(
                self.fence.host_generation,
                request_id,
                "storage_request_timeout",
                "MiniApp Service storage request timed out",
            );
            if let Err(error) = write_json_line(&mut self.stdin, &response).await {
                return Some(ActorExit::Failed(error));
            }
        }
        None
    }

    fn fail_pending(&mut self, error: MiniAppServiceProcessError) {
        for (_, pending) in std::mem::take(&mut self.pending) {
            match pending.reply {
                PendingReply::Invoke(reply) => {
                    let _ = reply.send(Err(clone_process_error(&error)));
                }
                PendingReply::Cancel | PendingReply::Stop => {}
            }
        }
        self.call_to_request.clear();
        self.retired_requests.clear();
        for pending in self.storage_pending.values() {
            pending.cancellation.cancel();
        }
        self.storage_pending.clear();
        while let Ok(command) = self.commands.try_recv() {
            match command {
                ActorCommand::Invoke { reply, .. } => {
                    let _ = reply.send(Err(clone_process_error(&error)));
                }
                ActorCommand::CancelCall { .. }
                | ActorCommand::StorageCompleted { .. }
                | ActorCommand::Stop => {}
            }
        }
    }
}

fn clone_process_error(error: &MiniAppServiceProcessError) -> MiniAppServiceProcessError {
    match error {
        MiniAppServiceProcessError::Rejected(message) => {
            MiniAppServiceProcessError::Rejected(message.clone())
        }
        MiniAppServiceProcessError::Crashed(message) => {
            MiniAppServiceProcessError::Crashed(message.clone())
        }
    }
}

fn storage_failure_response(
    host_generation: u64,
    request_id: String,
    code: &str,
    message: &str,
) -> ServiceStorageResponseFrame {
    ServiceStorageResponseFrame {
        kind: "storage_response",
        protocol_version: SERVICE_PROTOCOL_VERSION,
        host_generation,
        request_id,
        outcome: "failure",
        value: None,
        error: Some(ServiceStorageWireError {
            code: code.to_owned(),
            message: message.to_owned(),
        }),
    }
}

struct ServiceProcessInner {
    fence: MiniAppServiceGenerationFence,
    commands: mpsc::Sender<ActorCommand>,
    limits: MiniAppServiceProcessLimits,
    stopped: AtomicBool,
    completion: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
}

pub struct NodeMiniAppServiceProcess {
    inner: Arc<ServiceProcessInner>,
}

impl std::fmt::Debug for NodeMiniAppServiceProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeMiniAppServiceProcess")
            .field("fence", &self.inner.fence)
            .finish_non_exhaustive()
    }
}

impl NodeMiniAppServiceProcess {
    pub async fn stop_with_result(&self) -> Result<(), String> {
        if self
            .inner
            .stopped
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = self.inner.commands.send(ActorCommand::Stop).await;
        }

        let mut completion = self.inner.completion.clone();
        let wait = async {
            loop {
                if let Some(result) = completion.borrow_and_update().clone() {
                    return result;
                }
                completion
                    .changed()
                    .await
                    .map_err(|_| "MiniApp Service completion channel closed".to_owned())?;
            }
        };
        tokio::time::timeout(
            self.inner
                .limits
                .shutdown_timeout
                .saturating_add(self.inner.limits.shutdown_timeout),
            wait,
        )
        .await
        .map_err(|_| "MiniApp Service stop timed out".to_owned())?
    }
}

#[async_trait]
impl MiniAppServiceProcess for NodeMiniAppServiceProcess {
    async fn invoke(
        &self,
        invocation: MiniAppServiceInvocation,
        cancellation: MiniAppCallCancellation,
    ) -> Result<StrictJsonValue, MiniAppServiceProcessError> {
        let call_id = invocation.call_id.as_ref().to_owned();
        let (reply, mut response) = oneshot::channel();
        self.inner
            .commands
            .send(ActorCommand::Invoke {
                invocation,
                cancellation: cancellation.clone(),
                reply,
            })
            .await
            .map_err(|_| {
                MiniAppServiceProcessError::Crashed(
                    "MiniApp Service command channel is closed".into(),
                )
            })?;
        loop {
            tokio::select! {
                result = &mut response => {
                    return result.unwrap_or_else(|_| {
                        Err(MiniAppServiceProcessError::Crashed(
                            "MiniApp Service response channel is closed".into(),
                        ))
                    });
                }
                _ = tokio::time::sleep(self.inner.limits.cancellation_poll_interval) => {
                    if cancellation.is_canceled() {
                        let _ = self.inner.commands.send(ActorCommand::CancelCall {
                            call_id: call_id.clone(),
                        }).await;
                        return Err(MiniAppServiceProcessError::Rejected(
                            "MiniApp Service call canceled".into(),
                        ));
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        let _ = self.stop_with_result().await;
    }

    fn terminal_result(&self) -> Option<Result<(), String>> {
        self.inner.completion.borrow().clone()
    }
}

async fn read_service_frames(
    mut reader: BufReader<ChildStdout>,
    sender: mpsc::Sender<ReaderEvent>,
    max_frame_bytes: usize,
) {
    loop {
        let frame = read_json_line::<serde_json::Value>(&mut reader, max_frame_bytes).await;
        let event = match frame {
            Ok(frame) => match frame.get("kind").and_then(serde_json::Value::as_str) {
                Some("response") => serde_json::from_value::<ServiceResponseFrame>(frame)
                    .map(|frame| ReaderEvent::Frame(ServiceInboundFrame::Response(frame)))
                    .unwrap_or_else(|error| ReaderEvent::Failed(error.to_string())),
                Some("storage_request") => {
                    serde_json::from_value::<ServiceStorageRequestFrame>(frame)
                        .map(|frame| ReaderEvent::Frame(ServiceInboundFrame::Storage(frame)))
                        .unwrap_or_else(|error| ReaderEvent::Failed(error.to_string()))
                }
                Some(kind) => ReaderEvent::Failed(format!(
                    "MiniApp Service emitted unsupported frame kind {kind}"
                )),
                None => ReaderEvent::Failed(
                    "MiniApp Service emitted a frame without kind".into(),
                ),
            },
            Err(error) if error == "MiniApp Service IPC reached EOF" => ReaderEvent::Eof,
            Err(error) => ReaderEvent::Failed(error),
        };
        let terminal = matches!(event, ReaderEvent::Eof | ReaderEvent::Failed(_));
        if sender.send(event).await.is_err() || terminal {
            return;
        }
    }
}

async fn read_service_hello(
    reader: &mut BufReader<ChildStdout>,
    stdin: &mut ChildStdin,
    spec: &ResolvedMiniAppServiceSpec,
    host_generation: u64,
    storage: Option<Arc<dyn MiniAppServiceStoragePort>>,
    limits: &MiniAppServiceProcessLimits,
) -> Result<ServiceHello, String> {
    loop {
        let frame = read_json_line::<serde_json::Value>(reader, limits.max_frame_bytes).await?;
        match frame.get("kind").and_then(serde_json::Value::as_str) {
            Some("hello") => {
                return serde_json::from_value(frame).map_err(|error| error.to_string());
            }
            Some("storage_request") => {
                let frame: ServiceStorageRequestFrame =
                    serde_json::from_value(frame).map_err(|error| error.to_string())?;
                if frame.protocol_version != SERVICE_PROTOCOL_VERSION
                    || frame.host_generation == 0
                    || frame.host_generation != host_generation
                {
                    return Err(
                        "MiniApp Service startup storage request identity is invalid".into(),
                    );
                }
                let request = decode_storage_request(&frame.operation, frame.payload)
                    .map_err(|error| format!("invalid startup storage request: {error}"))?;
                let result = match storage.as_ref() {
                    Some(storage) => storage
                        .handle_service_request(
                            &spec.miniapp_id,
                            &spec.storage,
                            request,
                            MiniAppCallCancellation::default(),
                        )
                        .await
                        .map_err(|error| error.to_string()),
                    None => Err("MiniApp managed Service storage is not configured".into()),
                };
                let response = match result {
                    Ok(value) => ServiceStorageResponseFrame {
                        kind: "storage_response",
                        protocol_version: SERVICE_PROTOCOL_VERSION,
                        host_generation: frame.host_generation,
                        request_id: frame.request_id,
                        outcome: "success",
                        value: Some(value),
                        error: None,
                    },
                    Err(message) => storage_failure_response(
                        frame.host_generation,
                        frame.request_id,
                        "storage_request_failed",
                        &message,
                    ),
                };
                write_json_line(stdin, &response).await?;
            }
            Some(kind) => {
                return Err(format!(
                    "MiniApp Service emitted {kind} before its Hello frame"
                ));
            }
            None => {
                return Err("MiniApp Service startup frame has no kind".into());
            }
        }
    }
}

fn decode_storage_request(
    operation: &str,
    payload: StrictJsonValue,
) -> Result<MiniAppServiceStorageRequest, String> {
    match operation {
        "kv" => serde_json::from_value(payload.0)
            .map(|request| MiniAppServiceStorageRequest::Kv { request })
            .map_err(|error| error.to_string()),
        "database_query" => serde_json::from_value(payload.0)
            .map(|statement| MiniAppServiceStorageRequest::DatabaseQuery { statement })
            .map_err(|error| error.to_string()),
        "database_execute" => serde_json::from_value(payload.0)
            .map(|statement| MiniAppServiceStorageRequest::DatabaseExecute { statement })
            .map_err(|error| error.to_string()),
        "database_batch" => {
            let object = payload
                .0
                .as_object()
                .ok_or_else(|| "database batch payload must be an object".to_owned())?;
            let statements = object
                .get("statements")
                .cloned()
                .ok_or_else(|| "database batch payload is missing statements".to_owned())?;
            serde_json::from_value(statements)
                .map(|statements| MiniAppServiceStorageRequest::DatabaseBatch { statements })
                .map_err(|error| error.to_string())
        }
        _ => Err(format!(
            "unsupported MiniApp Service storage operation {operation}"
        )),
    }
}

async fn read_json_line<T: for<'de> Deserialize<'de>>(
    reader: &mut (impl AsyncBufRead + Unpin),
    max_frame_bytes: usize,
) -> Result<T, String> {
    let mut frame = Vec::new();
    let limit = u64::try_from(max_frame_bytes.saturating_add(1)).unwrap_or(u64::MAX);
    let mut limited = reader.take(limit);
    let read = limited
        .read_until(b'\n', &mut frame)
        .await
        .map_err(|error| error.to_string())?;
    if read == 0 {
        return Err("MiniApp Service IPC reached EOF".into());
    }
    if frame.len() > max_frame_bytes {
        return Err(format!(
            "MiniApp Service IPC frame exceeds {max_frame_bytes} bytes"
        ));
    }
    while matches!(frame.last(), Some(b'\n' | b'\r')) {
        frame.pop();
    }
    serde_json::from_slice(&frame).map_err(|error| error.to_string())
}

async fn write_json_line<T: Serialize>(
    stdin: &mut ChildStdin,
    value: &T,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| format!("MiniApp Service IPC write failed: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("MiniApp Service IPC flush failed: {error}"))
}

#[derive(Clone, Copy, Debug, Default)]
struct StderrSummary {
    bytes: u64,
    lines: u64,
}

async fn drain_stderr(mut stderr: ChildStderr) -> StderrSummary {
    let mut summary = StderrSummary::default();
    let mut buffer = [0_u8; 8192];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return summary,
            Ok(read) => {
                summary.bytes = summary
                    .bytes
                    .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                summary.lines = summary.lines.saturating_add(
                    u64::try_from(buffer[..read].iter().filter(|byte| **byte == b'\n').count())
                        .unwrap_or(u64::MAX),
                );
            }
        }
    }
}
