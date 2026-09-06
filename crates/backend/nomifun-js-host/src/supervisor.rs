use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_contracts::{
    ActionId, CanonicalErrorCode, CanonicalSchemaRef, CorrelationId, DigestHex,
    JAVASCRIPT_HOST_PROTOCOL_VERSION, JavaScriptHostHello, JavaScriptHostKind,
    JavaScriptHostMessageDirection, JavaScriptHostMethod, NodeRuntimeFingerprint,
    PluginHostCommitFence, PluginHostContributionRef, PluginHostRequest,
    PluginHostRequestEnvelope, PluginHostResponseBody, PluginHostResponseEnvelope,
    PluginHostSuccess, PluginHostTargetLock, PluginHostWireError,
    PluginMountId, PluginMountRuntimeContext, PluginN1ContractManifest,
    ResourceBindingId, ResourceKind, StrictJsonValue, VersionString, digest_payload,
};
use nomifun_js_runtime::{NodeProbeCandidate, NodeRuntimeResolver};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader,
};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::warn;
use uuid::Uuid;

use crate::JavaScriptHostError;

const HOST_FAILURE_CODE: &str = "JAVASCRIPT_HOST_UNAVAILABLE";
const HOST_SERVICE_UNAVAILABLE: &str = "HOST_SERVICE_UNAVAILABLE";

#[derive(Clone, Debug)]
pub struct JavaScriptHostLimits {
    pub hello_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_frame_bytes: usize,
    pub command_queue_capacity: usize,
}

impl Default for JavaScriptHostLimits {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(5),
            max_frame_bytes: 4 * 1024 * 1024,
            command_queue_capacity: 256,
        }
    }
}

impl JavaScriptHostLimits {
    fn validate(&self) -> Result<(), JavaScriptHostError> {
        if self.hello_timeout.is_zero()
            || self.request_timeout.is_zero()
            || self.shutdown_timeout.is_zero()
            || self.max_frame_bytes == 0
            || self.command_queue_capacity == 0
        {
            return Err(JavaScriptHostError::InvalidConfiguration(
                "timeouts, frame size, and queue capacity must be non-zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct JavaScriptHostConfig {
    pub node_executable: PathBuf,
    pub runtime: NodeRuntimeFingerprint,
    pub host_module: PathBuf,
    pub limits: JavaScriptHostLimits,
}

impl JavaScriptHostConfig {
    pub fn bundled(
        node_executable: PathBuf,
        runtime: NodeRuntimeFingerprint,
    ) -> Self {
        Self {
            node_executable,
            runtime,
            host_module: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join("extension-host.mjs"),
            limits: JavaScriptHostLimits::default(),
        }
    }

    fn validate_paths(&self) -> Result<(), JavaScriptHostError> {
        self.limits.validate()?;
        self.runtime
            .validate()
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        for (field, path) in [
            ("node_executable", &self.node_executable),
            ("host_module", &self.host_module),
        ] {
            if !path.is_absolute() {
                return Err(JavaScriptHostError::InvalidConfiguration(format!(
                    "{field} must be absolute"
                )));
            }
            if !path.is_file() {
                return Err(JavaScriptHostError::InvalidConfiguration(format!(
                    "{field} does not exist: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImmutablePluginModule {
    pub main_mjs: PathBuf,
    pub module_digest: DigestHex,
    pub target: PluginHostTargetLock,
}

impl ImmutablePluginModule {
    pub fn new(
        main_mjs: PathBuf,
        module_digest: DigestHex,
        target: PluginHostTargetLock,
    ) -> Self {
        Self {
            main_mjs,
            module_digest,
            target,
        }
    }

    async fn verify_for(
        &self,
        expected_target: &PluginHostTargetLock,
    ) -> Result<PathBuf, JavaScriptHostError> {
        self.target
            .validate()
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        if &self.target != expected_target {
            return Err(JavaScriptHostError::TargetMismatch);
        }
        if !self.main_mjs.is_absolute()
            || self.main_mjs.file_name().and_then(|name| name.to_str())
                != Some("main.mjs")
        {
            return Err(JavaScriptHostError::InvalidModule(
                "entrypoint must be an absolute path ending in main.mjs".into(),
            ));
        }
        let canonical = tokio::fs::canonicalize(&self.main_mjs)
            .await
            .map_err(|error| JavaScriptHostError::InvalidModule(error.to_string()))?;
        let canonical = dunce::simplified(&canonical).to_path_buf();
        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(|error| JavaScriptHostError::InvalidModule(error.to_string()))?;
        let observed = hex::encode(Sha256::digest(bytes));
        if observed != self.module_digest.as_ref() {
            return Err(JavaScriptHostError::InvalidModule(format!(
                "main.mjs digest mismatch: expected {}, observed {observed}",
                self.module_digest.as_ref()
            )));
        }
        Ok(canonical)
    }
}

#[derive(Clone, Debug)]
pub struct MountLoadDemand {
    pub context: PluginMountRuntimeContext,
    pub module: ImmutablePluginModule,
}

#[derive(Clone, Debug)]
pub struct BoundHostServiceRequest {
    pub mount: PluginMountRuntimeContext,
    pub envelope: PluginHostRequestEnvelope,
}

#[async_trait]
pub trait ExtensionHostServices: Send + Sync {
    async fn handle(
        &self,
        request: BoundHostServiceRequest,
    ) -> PluginHostResponseBody;
}

#[derive(Clone, Debug, Default)]
pub struct DenyExtensionHostServices;

#[async_trait]
impl ExtensionHostServices for DenyExtensionHostServices {
    async fn handle(
        &self,
        _request: BoundHostServiceRequest,
    ) -> PluginHostResponseBody {
        PluginHostResponseBody::Failure(PluginHostWireError {
            code: CanonicalErrorCode::from(HOST_SERVICE_UNAVAILABLE),
            message: "the requested Host service is not configured".into(),
            retryable: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JavaScriptHostState {
    Stopped,
    Running {
        generation: u64,
        process_id: u32,
    },
    Failed {
        generation: u64,
        reason: String,
    },
}

#[derive(Debug)]
pub struct HostRequestHandle {
    pub host_generation: u64,
    pub request_id: CorrelationId,
    response: oneshot::Receiver<Result<PluginHostSuccess, JavaScriptHostError>>,
}

impl HostRequestHandle {
    pub async fn wait(self) -> Result<PluginHostSuccess, JavaScriptHostError> {
        self.response
            .await
            .unwrap_or(Err(JavaScriptHostError::RequestChannelClosed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaScriptResourceHandle {
    pub host_generation: u64,
    pub handle_id: String,
}

pub struct ExtensionHostSupervisor {
    config: JavaScriptHostConfig,
    contract: PluginN1ContractManifest,
    services: Arc<dyn ExtensionHostServices>,
    state: Mutex<SupervisorState>,
    public_state: watch::Sender<JavaScriptHostState>,
}

struct SupervisorState {
    next_generation: u64,
    current: Option<GenerationHandle>,
}

#[derive(Clone)]
struct GenerationHandle {
    generation: u64,
    commands: mpsc::Sender<ActorCommand>,
    state: watch::Receiver<JavaScriptHostState>,
    mounts: Arc<RwLock<BTreeMap<PluginMountId, PluginMountRuntimeContext>>>,
    admission: Arc<RwLock<()>>,
}

impl ExtensionHostSupervisor {
    pub fn new(config: JavaScriptHostConfig) -> Result<Self, JavaScriptHostError> {
        Self::with_services(config, Arc::new(DenyExtensionHostServices))
    }

    pub fn with_services(
        config: JavaScriptHostConfig,
        services: Arc<dyn ExtensionHostServices>,
    ) -> Result<Self, JavaScriptHostError> {
        config.validate_paths()?;
        let contract = PluginN1ContractManifest::canonical();
        contract
            .validate()
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        let (public_state, _) = watch::channel(JavaScriptHostState::Stopped);
        Ok(Self {
            config,
            contract,
            services,
            state: Mutex::new(SupervisorState {
                next_generation: 0,
                current: None,
            }),
            public_state,
        })
    }

    pub fn subscribe_state(&self) -> watch::Receiver<JavaScriptHostState> {
        self.public_state.subscribe()
    }

    pub fn state(&self) -> JavaScriptHostState {
        self.public_state.borrow().clone()
    }

    pub fn process_count(&self) -> usize {
        usize::from(matches!(
            self.public_state.borrow().clone(),
            JavaScriptHostState::Running { .. }
        ))
    }

    pub async fn load_mount(
        &self,
        demand: MountLoadDemand,
    ) -> Result<u64, JavaScriptHostError> {
        demand
            .context
            .validate()
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        let module_path = demand.module.verify_for(&demand.context.target).await?;
        let handle = self.ensure_generation().await?;
        if let Some(resident) = handle
            .mounts
            .read()
            .await
            .get(&demand.context.target.mount_id)
        {
            return if resident == &demand.context {
                Ok(handle.generation)
            } else {
                Err(JavaScriptHostError::TargetMismatch)
            };
        }
        let request = PluginHostRequest::MountLoad {
            context: demand.context,
        };
        let response = handle
            .request(
                &self.contract,
                request,
                Some(module_path),
                self.config.limits.request_timeout,
            )
            .await?
            .wait()
            .await?;
        require_ack(response)?;
        Ok(handle.generation)
    }

    pub async fn unload_mount(
        &self,
        target: PluginHostTargetLock,
    ) -> Result<(), JavaScriptHostError> {
        let handle = self.running_generation().await?;
        let response = handle
            .request(
                &self.contract,
                PluginHostRequest::MountUnload { target },
                None,
                self.config.limits.request_timeout,
            )
            .await?
            .wait()
            .await?;
        require_ack(response)
    }

    pub async fn start_invocation(
        &self,
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    ) -> Result<HostRequestHandle, JavaScriptHostError> {
        let handle = self
            .resident_generation_for(&contribution)
            .await?;
        handle
            .request(
                &self.contract,
                PluginHostRequest::CapabilityInvoke {
                    contribution,
                    action_id,
                    input,
                },
                None,
                self.config.limits.request_timeout,
            )
            .await
    }

    pub async fn invoke(
        &self,
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        match self
            .start_invocation(contribution, action_id, input)
            .await?
            .wait()
            .await?
        {
            PluginHostSuccess::Value(value) => Ok(value),
            _ => Err(JavaScriptHostError::Contract(
                "CapabilityInvoke returned a non-value success".into(),
            )),
        }
    }

    pub async fn invoke_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        if mount.context.target != contribution.target {
            return Err(JavaScriptHostError::TargetMismatch);
        }
        self.load_mount(mount).await?;
        self.invoke(contribution, action_id, input).await
    }

    pub async fn contribute_context(
        &self,
        contribution: PluginHostContributionRef,
        schema_ref: CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        let handle = self
            .resident_generation_for(&contribution)
            .await?;
        match handle
            .request(
                &self.contract,
                PluginHostRequest::ContextContribute {
                    contribution,
                    schema_ref,
                },
                None,
                self.config.limits.request_timeout,
            )
            .await?
            .wait()
            .await?
        {
            PluginHostSuccess::Value(value) => Ok(value),
            _ => Err(JavaScriptHostError::Contract(
                "ContextContribute returned a non-value success".into(),
            )),
        }
    }

    pub async fn contribute_context_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        schema_ref: CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        if mount.context.target != contribution.target {
            return Err(JavaScriptHostError::TargetMismatch);
        }
        self.load_mount(mount).await?;
        self.contribute_context(contribution, schema_ref).await
    }

    pub async fn acquire_resource(
        &self,
        contribution: PluginHostContributionRef,
        binding_id: ResourceBindingId,
        resource_kind: ResourceKind,
        parameters: StrictJsonValue,
    ) -> Result<JavaScriptResourceHandle, JavaScriptHostError> {
        let handle = self
            .resident_generation_for(&contribution)
            .await?;
        let host_generation = handle.generation;
        match handle
            .request(
                &self.contract,
                PluginHostRequest::ResourceAcquire {
                    contribution,
                    binding_id,
                    resource_kind,
                    parameters,
                },
                None,
                self.config.limits.request_timeout,
            )
            .await?
            .wait()
            .await?
        {
            PluginHostSuccess::ResourceAcquired { handle_id } => {
                Ok(JavaScriptResourceHandle {
                    host_generation,
                    handle_id,
                })
            }
            _ => Err(JavaScriptHostError::Contract(
                "ResourceAcquire returned a non-resource success".into(),
            )),
        }
    }

    pub async fn acquire_resource_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        binding_id: ResourceBindingId,
        resource_kind: ResourceKind,
        parameters: StrictJsonValue,
    ) -> Result<JavaScriptResourceHandle, JavaScriptHostError> {
        if mount.context.target != contribution.target {
            return Err(JavaScriptHostError::TargetMismatch);
        }
        self.load_mount(mount).await?;
        self.acquire_resource(
            contribution,
            binding_id,
            resource_kind,
            parameters,
        )
        .await
    }

    pub async fn release_resource(
        &self,
        resource: &JavaScriptResourceHandle,
    ) -> Result<(), JavaScriptHostError> {
        let handle = {
            let state = self.state.lock().await;
            let Some(current) = &state.current else {
                return Ok(());
            };
            if current.generation != resource.host_generation
                || !matches!(
                    current.state.borrow().clone(),
                    JavaScriptHostState::Running { .. }
                )
            {
                return Ok(());
            }
            current.clone()
        };
        let response = handle
            .request(
                &self.contract,
                PluginHostRequest::ResourceRelease {
                    handle_id: resource.handle_id.clone(),
                },
                None,
                self.config.limits.request_timeout,
            )
            .await?
            .wait()
            .await?;
        require_ack(response)
    }

    pub async fn cancel(
        &self,
        request_id: CorrelationId,
    ) -> Result<(), JavaScriptHostError> {
        let handle = self.running_generation().await?;
        let response = handle
            .request(
                &self.contract,
                PluginHostRequest::RequestCancel {
                    target_request_id: request_id,
                },
                None,
                self.config.limits.request_timeout,
            )
            .await?
            .wait()
            .await?;
        require_ack(response)
    }

    pub async fn commit_fence_for_mount(
        &self,
        mount_id: &PluginMountId,
    ) -> Result<PluginHostCommitFence, JavaScriptHostError> {
        let state = self.state.lock().await;
        let Some(current) = &state.current else {
            return Ok(PluginHostCommitFence::NotResident);
        };
        if !matches!(
            current.state.borrow().clone(),
            JavaScriptHostState::Running { .. }
        ) || !current.mounts.read().await.contains_key(mount_id)
        {
            return Ok(PluginHostCommitFence::NotResident);
        }
        Err(JavaScriptHostError::NotQuiescent {
            generation: current.generation,
        })
    }

    pub async fn stop_generation(
        &self,
        expected_generation: u64,
    ) -> Result<PluginHostCommitFence, JavaScriptHostError> {
        let handle = {
            let state = self.state.lock().await;
            let Some(current) = &state.current else {
                return Ok(PluginHostCommitFence::NotResident);
            };
            if current.generation != expected_generation {
                return Err(JavaScriptHostError::GenerationMismatch {
                    expected: expected_generation,
                    observed: current.generation,
                });
            }
            current.clone()
        };
        let _admission = handle.admission.write().await;
        if !matches!(
            handle.state.borrow().clone(),
            JavaScriptHostState::Running { .. }
        ) {
            return Err(JavaScriptHostError::HostFailure {
                generation: expected_generation,
                reason: "Host generation stopped before the quiescent fence".into(),
            });
        }
        let (reply, response) = oneshot::channel();
        handle
            .commands
            .send(ActorCommand::Stop {
                expected_generation,
                reply,
            })
            .await
            .map_err(|_| JavaScriptHostError::HostFailure {
                generation: expected_generation,
                reason: "Host command queue is closed".into(),
            })?;
        response
            .await
            .unwrap_or(Err(JavaScriptHostError::RequestChannelClosed))
    }

    async fn resident_generation_for(
        &self,
        contribution: &PluginHostContributionRef,
    ) -> Result<GenerationHandle, JavaScriptHostError> {
        contribution
            .validate()
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        let handle = self.ensure_generation().await?;
        let resident = handle.mounts.read().await;
        let Some(mount) = resident.get(&contribution.target.mount_id) else {
            return Err(JavaScriptHostError::MountNotResident(
                contribution.target.mount_id.as_ref().to_owned(),
            ));
        };
        if mount.target != contribution.target {
            return Err(JavaScriptHostError::TargetMismatch);
        }
        drop(resident);
        Ok(handle)
    }

    async fn running_generation(
        &self,
    ) -> Result<GenerationHandle, JavaScriptHostError> {
        let state = self.state.lock().await;
        let Some(current) = &state.current else {
            return Err(JavaScriptHostError::HostFailure {
                generation: 0,
                reason: "no Host generation is running".into(),
            });
        };
        if !matches!(
            current.state.borrow().clone(),
            JavaScriptHostState::Running { .. }
        ) {
            return Err(JavaScriptHostError::HostFailure {
                generation: current.generation,
                reason: "the current Host generation is not running".into(),
            });
        }
        Ok(current.clone())
    }

    async fn ensure_generation(
        &self,
    ) -> Result<GenerationHandle, JavaScriptHostError> {
        let mut state = self.state.lock().await;
        if let Some(current) = &state.current
            && matches!(
                current.state.borrow().clone(),
                JavaScriptHostState::Running { .. }
            )
        {
            return Ok(current.clone());
        }

        state.next_generation = state
            .next_generation
            .checked_add(1)
            .ok_or_else(|| {
                JavaScriptHostError::InvalidConfiguration(
                    "Host generation counter overflowed".into(),
                )
            })?;
        let generation = state.next_generation;
        let handle = spawn_generation(
            generation,
            &self.config,
            &self.contract,
            Arc::clone(&self.services),
            self.public_state.clone(),
        )
        .await?;
        state.current = Some(handle.clone());
        Ok(handle)
    }
}

impl GenerationHandle {
    async fn request(
        &self,
        contract: &PluginN1ContractManifest,
        request: PluginHostRequest,
        module_path: Option<PathBuf>,
        timeout: Duration,
    ) -> Result<HostRequestHandle, JavaScriptHostError> {
        let _admission = self.admission.read().await;
        if !matches!(
            self.state.borrow().clone(),
            JavaScriptHostState::Running { .. }
        ) {
            return Err(JavaScriptHostError::HostFailure {
                generation: self.generation,
                reason: "Host generation is not running".into(),
            });
        }
        let request_id = CorrelationId::from(Uuid::now_v7().to_string());
        let envelope = PluginHostRequestEnvelope {
            protocol_version: VersionString::from(
                JAVASCRIPT_HOST_PROTOCOL_VERSION,
            ),
            host_kind: JavaScriptHostKind::SharedExtension,
            host_generation: self.generation,
            request_id: request_id.clone(),
            direction: request.direction(),
            request,
        };
        envelope
            .validate(contract)
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ActorCommand::Submit {
                envelope: Box::new(envelope),
                module_path,
                deadline: Instant::now() + timeout,
                reply,
            })
            .await
            .map_err(|_| JavaScriptHostError::HostFailure {
                generation: self.generation,
                reason: "Host command queue is closed".into(),
            })?;
        Ok(HostRequestHandle {
            host_generation: self.generation,
            request_id,
            response,
        })
    }
}

fn require_ack(success: PluginHostSuccess) -> Result<(), JavaScriptHostError> {
    match success {
        PluginHostSuccess::Ack => Ok(()),
        _ => Err(JavaScriptHostError::Contract(
            "Host operation returned a non-ack success".into(),
        )),
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct HostBootstrap<'a> {
    host_kind: JavaScriptHostKind,
    host_generation: u64,
    runtime: &'a NodeRuntimeFingerprint,
    supported_methods: BTreeSet<JavaScriptHostMethod>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct PrivateRequestFrame<'a> {
    kind: &'static str,
    envelope: &'a PluginHostRequestEnvelope,
    #[serde(skip_serializing_if = "Option::is_none")]
    module_path: Option<&'a Path>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JavaScriptFrame {
    Response(PluginHostResponseEnvelope),
    Request(PluginHostRequestEnvelope),
}

enum ReaderEvent {
    Frame(Box<JavaScriptFrame>),
    Eof,
    Failed(String),
}

enum ActorCommand {
    Submit {
        envelope: Box<PluginHostRequestEnvelope>,
        module_path: Option<PathBuf>,
        deadline: Instant,
        reply: oneshot::Sender<
            Result<PluginHostSuccess, JavaScriptHostError>,
        >,
    },
    Stop {
        expected_generation: u64,
        reply: oneshot::Sender<
            Result<PluginHostCommitFence, JavaScriptHostError>,
        >,
    },
}

enum InternalEvent {
    HostServiceCompleted {
        request: PluginHostRequestEnvelope,
        response: PluginHostResponseBody,
    },
}

struct PendingRequest {
    envelope: PluginHostRequestEnvelope,
    deadline: Instant,
    reply: PendingReply,
}

enum PendingReply {
    Request(
        oneshot::Sender<Result<PluginHostSuccess, JavaScriptHostError>>,
    ),
    Stop(
        oneshot::Sender<
            Result<PluginHostCommitFence, JavaScriptHostError>,
        >,
    ),
}

struct GenerationActor {
    generation: u64,
    process_id: u32,
    process: ManagedChildProcess,
    stdin: ChildStdin,
    reader_events: mpsc::Receiver<ReaderEvent>,
    commands: mpsc::Receiver<ActorCommand>,
    internal_events: mpsc::Receiver<InternalEvent>,
    internal_sender: mpsc::Sender<InternalEvent>,
    pending: HashMap<CorrelationId, PendingRequest>,
    mounts: Arc<RwLock<BTreeMap<PluginMountId, PluginMountRuntimeContext>>>,
    services: Arc<dyn ExtensionHostServices>,
    contract: PluginN1ContractManifest,
    limits: JavaScriptHostLimits,
    state: watch::Sender<JavaScriptHostState>,
    public_state: watch::Sender<JavaScriptHostState>,
    reader_task: JoinHandle<()>,
    stderr_task: JoinHandle<StderrSummary>,
    accepting: bool,
}

enum ActorExit {
    Failed(String),
    Stopped {
        stop_reply: oneshot::Sender<
            Result<PluginHostCommitFence, JavaScriptHostError>,
        >,
    },
}

async fn spawn_generation(
    generation: u64,
    config: &JavaScriptHostConfig,
    contract: &PluginN1ContractManifest,
    services: Arc<dyn ExtensionHostServices>,
    public_state: watch::Sender<JavaScriptHostState>,
) -> Result<GenerationHandle, JavaScriptHostError> {
    verify_runtime(config).await?;
    let supported_methods = contract.host_method_sets
        [&JavaScriptHostKind::SharedExtension]
        .clone();
    let bootstrap = serde_json::to_vec(&HostBootstrap {
        host_kind: JavaScriptHostKind::SharedExtension,
        host_generation: generation,
        runtime: &config.runtime,
        supported_methods,
    })
    .map_err(|error| JavaScriptHostError::Spawn(error.to_string()))?;
    let bootstrap = hex::encode(bootstrap);

    let mut builder = ChildProcessBuilder::new(&config.node_executable);
    let host_module = dunce::simplified(&config.host_module);
    builder
        .arg(host_module)
        .arg(bootstrap)
        .current_dir(
            host_module.parent().ok_or_else(|| {
                JavaScriptHostError::InvalidConfiguration(
                    "host_module has no parent directory".into(),
                )
            })?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = builder
        .spawn_managed()
        .map_err(|error| JavaScriptHostError::Spawn(error.to_string()))?;
    let process_id = process.id().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node process did not expose a process id".into())
    })?;
    let stdin = process.stdin.take().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node stdin was not captured".into())
    })?;
    let stdout = process.stdout.take().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node stdout was not captured".into())
    })?;
    let mut stderr = process.stderr.take().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node stderr was not captured".into())
    })?;

    let mut reader = BufReader::new(stdout);
    let hello = match tokio::time::timeout(
        config.limits.hello_timeout,
        read_json_line::<JavaScriptHostHello>(&mut reader, config.limits.max_frame_bytes),
    )
    .await
    {
        Ok(Ok(hello)) => hello,
        Ok(Err(error)) => {
            let detail =
                startup_failure_detail(&mut process, &mut stderr).await;
            return Err(JavaScriptHostError::HelloRejected(format!(
                "{error}{detail}"
            )));
        }
        Err(_) => {
            let detail =
                startup_failure_detail(&mut process, &mut stderr).await;
            return Err(JavaScriptHostError::HelloRejected(format!(
                "Hello timed out{detail}"
            )));
        }
    };
    if let Err(error) = hello.validate(contract) {
        let detail = startup_failure_detail(&mut process, &mut stderr).await;
        return Err(JavaScriptHostError::HelloRejected(format!(
            "{error}{detail}"
        )));
    }
    if hello.host_kind != JavaScriptHostKind::SharedExtension
        || hello.host_generation != generation
        || hello.process_id != process_id
        || hello.runtime != config.runtime
    {
        let detail = startup_failure_detail(&mut process, &mut stderr).await;
        return Err(JavaScriptHostError::HelloRejected(
            format!(
                "Hello does not bind the selected Runtime, role, generation, and process{detail}"
            ),
        ));
    }

    let (reader_sender, reader_events) =
        mpsc::channel(config.limits.command_queue_capacity);
    let reader_task = tokio::spawn(read_frames(
        reader,
        reader_sender,
        config.limits.max_frame_bytes,
    ));
    let stderr_task = tokio::spawn(drain_stderr(stderr));
    let (command_sender, commands) =
        mpsc::channel(config.limits.command_queue_capacity);
    let (internal_sender, internal_events) =
        mpsc::channel(config.limits.command_queue_capacity);
    let running = JavaScriptHostState::Running {
        generation,
        process_id,
    };
    let (state, state_rx) = watch::channel(running.clone());
    public_state.send_replace(running);
    let mounts = Arc::new(RwLock::new(BTreeMap::new()));
    let admission = Arc::new(RwLock::new(()));

    let actor = GenerationActor {
        generation,
        process_id,
        process,
        stdin,
        reader_events,
        commands,
        internal_events,
        internal_sender,
        pending: HashMap::new(),
        mounts: Arc::clone(&mounts),
        services,
        contract: contract.clone(),
        limits: config.limits.clone(),
        state,
        public_state,
        reader_task,
        stderr_task,
        accepting: true,
    };
    tokio::spawn(actor.run());

    Ok(GenerationHandle {
        generation,
        commands: command_sender,
        state: state_rx,
        mounts,
        admission,
    })
}

async fn verify_runtime(
    config: &JavaScriptHostConfig,
) -> Result<(), JavaScriptHostError> {
    let resolver = NodeRuntimeResolver::default();
    let probe = resolver
        .probe(&NodeProbeCandidate::new(
            config.runtime.source_kind,
            config.node_executable.clone(),
        ))
        .await;
    match probe.fingerprint {
        Some(observed) if observed == config.runtime => Ok(()),
        Some(observed) => Err(JavaScriptHostError::RuntimeVerification(format!(
            "selected fingerprint {:?} differs from observed {:?}",
            config.runtime.runtime_installation_id,
            observed.runtime_installation_id
        ))),
        None => Err(JavaScriptHostError::RuntimeVerification(format!(
            "Node probe failed with {:?}",
            probe.error_code
        ))),
    }
}

async fn startup_failure_detail(
    process: &mut ManagedChildProcess,
    stderr: &mut ChildStderr,
) -> String {
    let _ = process.shutdown().await;
    let mut bytes = Vec::new();
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        stderr.take(4096).read_to_end(&mut bytes),
    )
    .await;
    let detail = String::from_utf8_lossy(&bytes).trim().to_owned();
    if detail.is_empty() {
        String::new()
    } else {
        format!("; stderr: {detail}")
    }
}

impl GenerationActor {
    async fn run(mut self) {
        let exit = self.run_loop().await;
        self.accepting = false;
        let failure = match &exit {
            ActorExit::Failed(reason) => JavaScriptHostError::HostFailure {
                generation: self.generation,
                reason: reason.clone(),
            },
            ActorExit::Stopped { .. } => JavaScriptHostError::HostStopping {
                generation: self.generation,
            },
        };
        self.fail_all(failure.clone());
        let cleanup = tokio::time::timeout(
            self.limits.shutdown_timeout,
            self.process.shutdown(),
        )
        .await;
        let cleanup_error = match cleanup {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error.to_string()),
            Err(_) => Some("process-tree shutdown timed out".into()),
        };
        let fence = self.commit_fence();
        self.reader_task.abort();
        let _ = self.reader_task.await;
        let stderr = self.stderr_task.await.unwrap_or_default();

        match exit {
            ActorExit::Stopped { stop_reply } if cleanup_error.is_none() => {
                self.state.send_replace(JavaScriptHostState::Stopped);
                self.public_state.send_replace(JavaScriptHostState::Stopped);
                let _ = stop_reply.send(Ok(fence));
            }
            ActorExit::Stopped { stop_reply } => {
                let reason = cleanup_error.unwrap();
                let failed = JavaScriptHostState::Failed {
                    generation: self.generation,
                    reason: reason.clone(),
                };
                self.state.send_replace(failed.clone());
                self.public_state.send_replace(failed);
                let _ = stop_reply.send(Err(
                    JavaScriptHostError::HostFailure {
                        generation: self.generation,
                        reason,
                    },
                ));
            }
            ActorExit::Failed(reason) => {
                let reason = if let Some(cleanup) = cleanup_error {
                    format!("{reason}; cleanup failed: {cleanup}")
                } else if stderr.bytes > 0 {
                    format!(
                        "{reason}; stderr contained {} bytes across {} lines",
                        stderr.bytes, stderr.lines
                    )
                } else {
                    reason
                };
                let failed = JavaScriptHostState::Failed {
                    generation: self.generation,
                    reason,
                };
                self.state.send_replace(failed.clone());
                self.public_state.send_replace(failed);
            }
        }
    }

    async fn run_loop(&mut self) -> ActorExit {
        let tick_period = self
            .limits
            .request_timeout
            .min(Duration::from_millis(50));
        let mut watchdog = tokio::time::interval(tick_period);
        watchdog.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        return ActorExit::Failed("all Host command senders were dropped".into());
                    };
                    if let Some(exit) = self.handle_command(command).await {
                        return exit;
                    }
                }
                event = self.reader_events.recv() => {
                    match event {
                        Some(ReaderEvent::Frame(frame)) => {
                            match self.handle_frame(*frame).await {
                                Ok(Some(exit)) => return exit,
                                Ok(None) => {}
                                Err(reason) => return ActorExit::Failed(reason),
                            }
                        }
                        Some(ReaderEvent::Eof) | None => {
                            return ActorExit::Failed("JavaScript Host IPC reached EOF".into());
                        }
                        Some(ReaderEvent::Failed(reason)) => {
                            return ActorExit::Failed(reason);
                        }
                    }
                }
                event = self.internal_events.recv() => {
                    if let Some(event) = event
                        && let Err(reason) = self.handle_internal(event).await
                    {
                        return ActorExit::Failed(reason);
                    }
                }
                _ = watchdog.tick() => {
                    if let Ok(Some(status)) = self.process.try_wait() {
                        return ActorExit::Failed(format!(
                            "JavaScript Host process exited with {status}"
                        ));
                    }
                    let now = Instant::now();
                    if let Some(expired) = self.pending.values().find(|pending| pending.deadline <= now) {
                        return ActorExit::Failed(format!(
                            "watchdog timed out request {}",
                            expired.envelope.request_id.as_ref()
                        ));
                    }
                }
            }
        }
    }

    async fn handle_command(
        &mut self,
        command: ActorCommand,
    ) -> Option<ActorExit> {
        match command {
            ActorCommand::Submit {
                envelope,
                module_path,
                deadline,
                reply,
            } => {
                if !self.accepting {
                    let _ = reply.send(Err(JavaScriptHostError::HostStopping {
                        generation: self.generation,
                    }));
                    return None;
                }
                if envelope.host_generation != self.generation {
                    let observed = envelope.host_generation;
                    let _ = reply.send(Err(
                        JavaScriptHostError::GenerationMismatch {
                            expected: self.generation,
                            observed,
                        },
                    ));
                    return None;
                }
                if let PluginHostRequest::MountLoad { context } = &envelope.request
                    && self.mount_identity_is_reserved(context).await
                {
                    let _ = reply.send(Err(JavaScriptHostError::Contract(
                        "Mount ID and mount_handle_id must be unique within one Host generation"
                            .into(),
                    )));
                    return None;
                }
                let request_id = envelope.request_id.clone();
                let frame = PrivateRequestFrame {
                    kind: "request",
                    envelope: envelope.as_ref(),
                    module_path: module_path.as_deref(),
                };
                if let Err(error) = write_json_line(&mut self.stdin, &frame).await
                {
                    let _ = reply.send(Err(JavaScriptHostError::HostFailure {
                        generation: self.generation,
                        reason: error.clone(),
                    }));
                    return Some(ActorExit::Failed(error));
                }
                self.pending.insert(
                    request_id,
                    PendingRequest {
                        envelope: *envelope,
                        deadline,
                        reply: PendingReply::Request(reply),
                    },
                );
                None
            }
            ActorCommand::Stop {
                expected_generation,
                reply,
            } => {
                if expected_generation != self.generation {
                    let _ = reply.send(Err(
                        JavaScriptHostError::GenerationMismatch {
                            expected: expected_generation,
                            observed: self.generation,
                        },
                    ));
                    return None;
                }
                if !self.pending.is_empty() {
                    let _ = reply.send(Err(JavaScriptHostError::NotQuiescent {
                        generation: self.generation,
                    }));
                    return None;
                }
                self.accepting = false;
                let request_id =
                    CorrelationId::from(Uuid::now_v7().to_string());
                let envelope = PluginHostRequestEnvelope {
                    protocol_version: VersionString::from(
                        JAVASCRIPT_HOST_PROTOCOL_VERSION,
                    ),
                    host_kind: JavaScriptHostKind::SharedExtension,
                    host_generation: self.generation,
                    request_id: request_id.clone(),
                    direction: JavaScriptHostMessageDirection::HostToJavaScript,
                    request: PluginHostRequest::HostShutdown,
                };
                let frame = PrivateRequestFrame {
                    kind: "request",
                    envelope: &envelope,
                    module_path: None,
                };
                if let Err(reason) = write_json_line(&mut self.stdin, &frame).await
                {
                    let _ = reply.send(Err(JavaScriptHostError::HostFailure {
                        generation: self.generation,
                        reason: reason.clone(),
                    }));
                    return Some(ActorExit::Failed(reason));
                }
                self.pending.insert(
                    request_id,
                    PendingRequest {
                        envelope,
                        deadline: Instant::now() + self.limits.shutdown_timeout,
                        reply: PendingReply::Stop(reply),
                    },
                );
                None
            }
        }
    }

    async fn handle_frame(
        &mut self,
        frame: JavaScriptFrame,
    ) -> Result<Option<ActorExit>, String> {
        match frame {
            JavaScriptFrame::Response(response) => {
                if response.host_generation != self.generation {
                    return Err(format!(
                        "rejected response from generation {} while generation {} is active",
                        response.host_generation, self.generation
                    ));
                }
                let pending = self
                    .pending
                    .remove(&response.request_id)
                    .ok_or_else(|| {
                        format!(
                            "rejected unknown or late response {}",
                            response.request_id.as_ref()
                        )
                    })?;
                response
                    .validate_for(&pending.envelope)
                    .map_err(|error| error.to_string())?;
                match pending.reply {
                    PendingReply::Request(reply) => {
                        let result = response_result(
                            &pending.envelope.request_id,
                            response.response,
                        );
                        if result.is_ok() {
                            self.apply_residency_change(&pending.envelope).await;
                        }
                        let _ = reply.send(result);
                        Ok(None)
                    }
                    PendingReply::Stop(reply) => match response.response {
                        PluginHostResponseBody::Success(
                            PluginHostSuccess::Ack,
                        ) => Ok(Some(ActorExit::Stopped {
                            stop_reply: reply,
                        })),
                        body => {
                            let error = response_result(
                                &pending.envelope.request_id,
                                body,
                            )
                            .err()
                            .unwrap_or_else(|| {
                                JavaScriptHostError::Contract(
                                    "HostShutdown did not return Ack".into(),
                                )
                            });
                            let _ = reply.send(Err(error.clone()));
                            Err(error.to_string())
                        }
                    },
                }
            }
            JavaScriptFrame::Request(request) => {
                if request.host_generation != self.generation {
                    return Err(format!(
                        "rejected Host service request from generation {}",
                        request.host_generation
                    ));
                }
                request
                    .validate(&self.contract)
                    .map_err(|error| error.to_string())?;
                if request.direction
                    != JavaScriptHostMessageDirection::JavaScriptToHost
                {
                    return Err(
                        "JavaScript emitted a request with the wrong direction"
                            .into(),
                    );
                }
                let mount_handle = host_service_mount_handle(&request.request)
                    .ok_or_else(|| {
                        "JavaScript emitted an unsupported Host service request"
                            .to_owned()
                    })?;
                let mount = self
                    .mounts
                    .read()
                    .await
                    .values()
                    .find(|mount| mount.mount_handle_id == mount_handle)
                    .cloned()
                    .ok_or_else(|| {
                        "JavaScript requested Host service for an unknown Mount handle"
                            .to_owned()
                    })?;
                let services = Arc::clone(&self.services);
                let sender = self.internal_sender.clone();
                tokio::spawn(async move {
                    let response = services
                        .handle(BoundHostServiceRequest {
                            mount,
                            envelope: request.clone(),
                        })
                        .await;
                    let _ = sender
                        .send(InternalEvent::HostServiceCompleted {
                            request,
                            response,
                        })
                        .await;
                });
                Ok(None)
            }
        }
    }

    async fn handle_internal(
        &mut self,
        event: InternalEvent,
    ) -> Result<(), String> {
        match event {
            InternalEvent::HostServiceCompleted { request, response } => {
                let envelope = PluginHostResponseEnvelope {
                    protocol_version: VersionString::from(
                        JAVASCRIPT_HOST_PROTOCOL_VERSION,
                    ),
                    host_kind: JavaScriptHostKind::SharedExtension,
                    host_generation: self.generation,
                    request_id: request.request_id.clone(),
                    response,
                };
                envelope
                    .validate_for(&request)
                    .map_err(|error| error.to_string())?;
                write_json_line(&mut self.stdin, &envelope).await
            }
        }
    }

    async fn apply_residency_change(
        &self,
        envelope: &PluginHostRequestEnvelope,
    ) {
        match &envelope.request {
            PluginHostRequest::MountLoad { context } => {
                self.mounts
                    .write()
                    .await
                    .insert(context.target.mount_id.clone(), context.clone());
            }
            PluginHostRequest::MountUnload { target } => {
                self.mounts.write().await.remove(&target.mount_id);
            }
            _ => {}
        }
    }

    async fn mount_identity_is_reserved(
        &self,
        context: &PluginMountRuntimeContext,
    ) -> bool {
        if self.mounts.read().await.iter().any(|(mount_id, resident)| {
            mount_id == &context.target.mount_id
                || resident.mount_handle_id == context.mount_handle_id
        }) {
            return true;
        }
        self.pending.values().any(|pending| {
            matches!(
                &pending.envelope.request,
                PluginHostRequest::MountLoad { context: pending_context }
                    if pending_context.target.mount_id == context.target.mount_id
                        || pending_context.mount_handle_id == context.mount_handle_id
            )
        })
    }

    fn fail_all(&mut self, error: JavaScriptHostError) {
        for (_, pending) in self.pending.drain() {
            match pending.reply {
                PendingReply::Request(reply) => {
                    let _ = reply.send(Err(error.clone()));
                }
                PendingReply::Stop(reply) => {
                    let _ = reply.send(Err(error.clone()));
                }
            }
        }
        while let Ok(command) = self.commands.try_recv() {
            match command {
                ActorCommand::Submit { reply, .. } => {
                    let _ = reply.send(Err(error.clone()));
                }
                ActorCommand::Stop { reply, .. } => {
                    let _ = reply.send(Err(error.clone()));
                }
            }
        }
    }

    fn commit_fence(&self) -> PluginHostCommitFence {
        #[derive(Serialize)]
        struct FenceInput {
            host_generation: u64,
            process_id: u32,
            state: &'static str,
        }
        let fence_token_digest = digest_payload(&FenceInput {
            host_generation: self.generation,
            process_id: self.process_id,
            state: "process_tree_stopped",
        })
        .expect("fixed Host fence input is serializable");
        PluginHostCommitFence::ResidentFenced {
            host_generation: self.generation,
            fence_token_digest,
        }
    }
}

fn response_result(
    request_id: &CorrelationId,
    response: PluginHostResponseBody,
) -> Result<PluginHostSuccess, JavaScriptHostError> {
    match response {
        PluginHostResponseBody::Success(success) => Ok(success),
        PluginHostResponseBody::Failure(error) => {
            Err(JavaScriptHostError::RequestFailed {
                request_id: request_id.clone(),
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            })
        }
    }
}

fn host_service_mount_handle(request: &PluginHostRequest) -> Option<&str> {
    match request {
        PluginHostRequest::CredentialResolve {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateGet {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateSet {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateDelete {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateCompareAndSwap {
            mount_handle_id, ..
        } => Some(mount_handle_id),
        _ => None,
    }
}

async fn write_json_line<T: Serialize>(
    stdin: &mut ChildStdin,
    value: &T,
) -> Result<(), String> {
    let mut encoded =
        serde_json::to_vec(value).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| format!("Host IPC write failed: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("Host IPC flush failed: {error}"))
}

async fn read_json_line<T: for<'de> Deserialize<'de>>(
    reader: &mut (impl AsyncBufRead + Unpin),
    max_frame_bytes: usize,
) -> Result<T, String> {
    let mut frame = Vec::new();
    let limit =
        u64::try_from(max_frame_bytes.saturating_add(1)).unwrap_or(u64::MAX);
    let mut limited = reader.take(limit);
    let read = limited
        .read_until(b'\n', &mut frame)
        .await
        .map_err(|error| error.to_string())?;
    if read == 0 {
        return Err("Host IPC reached EOF".into());
    }
    if frame.len() > max_frame_bytes {
        return Err(format!(
            "Host IPC frame exceeds {max_frame_bytes} bytes"
        ));
    }
    while matches!(frame.last(), Some(b'\n' | b'\r')) {
        frame.pop();
    }
    serde_json::from_slice(&frame).map_err(|error| error.to_string())
}

async fn read_frames(
    mut reader: BufReader<ChildStdout>,
    sender: mpsc::Sender<ReaderEvent>,
    max_frame_bytes: usize,
) {
    loop {
        let frame =
            read_json_line::<Value>(&mut reader, max_frame_bytes).await;
        let event = match frame {
            Ok(value) => match serde_json::from_value::<JavaScriptFrame>(value) {
                Ok(frame) => ReaderEvent::Frame(Box::new(frame)),
                Err(error) => ReaderEvent::Failed(format!(
                    "Host IPC emitted an invalid frame: {error}"
                )),
            },
            Err(error) if error == "Host IPC reached EOF" => ReaderEvent::Eof,
            Err(error) => ReaderEvent::Failed(error),
        };
        let terminal =
            matches!(event, ReaderEvent::Eof | ReaderEvent::Failed(_));
        if sender.send(event).await.is_err() || terminal {
            return;
        }
    }
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
            Ok(0) => return summary,
            Ok(read) => {
                summary.bytes = summary
                    .bytes
                    .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                summary.lines = summary.lines.saturating_add(
                    u64::try_from(
                        buffer[..read]
                            .iter()
                            .filter(|byte| **byte == b'\n')
                            .count(),
                    )
                    .unwrap_or(u64::MAX),
                );
            }
            Err(error) => {
                warn!(%error, "failed to drain JavaScript Host stderr");
                return summary;
            }
        }
    }
}

pub fn host_failure_response(reason: impl Into<String>) -> PluginHostResponseBody {
    PluginHostResponseBody::Failure(PluginHostWireError {
        code: CanonicalErrorCode::from(HOST_FAILURE_CODE),
        message: reason.into(),
        retryable: true,
    })
}
