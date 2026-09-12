use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_process_runtime::{ChildProcessBuilder, ChildProcessCleanup, ManagedChildProcess};
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
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader,
};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::warn;
use uuid::Uuid;

use crate::JavaScriptHostError;
use crate::outbound::OutboundQueue;

const HOST_SERVICE_UNAVAILABLE: &str = "HOST_SERVICE_UNAVAILABLE";
const BUNDLED_EXTENSION_HOST: &[u8] =
    include_bytes!("../assets/extension-host.mjs");

#[derive(Clone, Debug)]
pub struct JavaScriptHostLimits {
    pub hello_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    /// Maximum encoded frame size in either direction, including the newline.
    pub max_frame_bytes: usize,
    /// Capacity of each command, inbound, and outbound queue. Outbound overflow
    /// rejects an unsent caller request; an undeliverable service reply fails
    /// the generation rather than silently dropping a completed service result.
    pub command_queue_capacity: usize,
    /// Maximum admitted work callers, including coalesced Mount loads.
    /// One additional in-flight cancellation is reserved independently.
    pub max_pending_requests: usize,
    /// Maximum service requests, from admission through response flush.
    pub max_service_requests: usize,
}

impl Default for JavaScriptHostLimits {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(5),
            max_frame_bytes: 4 * 1024 * 1024,
            command_queue_capacity: 256,
            max_pending_requests: 256,
            max_service_requests: 256,
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
            || self.max_pending_requests == 0
            || self.max_service_requests == 0
        {
            return Err(JavaScriptHostError::InvalidConfiguration(
                "timeouts, frame size, and request/queue capacities must be non-zero".into(),
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
    pub fn for_host_module(
        node_executable: PathBuf,
        runtime: NodeRuntimeFingerprint,
        host_module: PathBuf,
    ) -> Self {
        Self {
            node_executable,
            runtime,
            host_module,
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

/// Publish the bundled entrypoint into an application-owned directory.
/// Concurrent publishers never expose a partially written destination; existing
/// files are verified, never overwritten. The directory must not be writable
/// by untrusted actors (this is not a sandbox against local filesystem mutation).
pub fn materialize_bundled_extension_host(
    directory: impl AsRef<Path>,
) -> Result<PathBuf, JavaScriptHostError> {
    use std::io::{Read, Write};

    let directory = directory.as_ref();
    fs_create_dir_all(directory)?;
    let io_error = |error: std::io::Error| {
        JavaScriptHostError::InvalidConfiguration(format!(
            "cannot materialize bundled JavaScript Host in {}: {error}",
            directory.display()
        ))
    };
    let directory = std::fs::canonicalize(directory).map_err(io_error)?;
    let digest = hex::encode(Sha256::digest(BUNDLED_EXTENSION_HOST));
    let path = directory.join(format!("extension-host-{digest}.mjs"));
    if !path.try_exists().map_err(io_error)? {
        // The temporary is on the same filesystem. Only complete, synced bytes
        // are published, without replacing another publisher's destination.
        let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(io_error)?;
        file.write_all(BUNDLED_EXTENSION_HOST).map_err(io_error)?;
        file.as_file().sync_all().map_err(io_error)?;
        match file.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error(error.error)),
        }
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(JavaScriptHostError::InvalidConfiguration(
            "bundled JavaScript Host path is not a regular file".into(),
        ));
    }
    // Do not allocate based on the size of a corrupt existing cache file.
    let mut observed = Vec::new();
    std::fs::File::open(&path).map_err(io_error)?
        .take(BUNDLED_EXTENSION_HOST.len() as u64 + 1)
        .read_to_end(&mut observed).map_err(io_error)?;
    if observed != BUNDLED_EXTENSION_HOST {
        return Err(JavaScriptHostError::InvalidConfiguration(
            "bundled JavaScript Host content differs from its digest path".into(),
        ));
    }
    let canonical = std::fs::canonicalize(&path).map_err(io_error)?;
    if !canonical.starts_with(&directory) {
        return Err(JavaScriptHostError::InvalidConfiguration(
            "bundled JavaScript Host escaped its managed directory".into(),
        ));
    }
    Ok(canonical)
}

fn fs_create_dir_all(path: &Path) -> Result<(), JavaScriptHostError> {
    std::fs::create_dir_all(path).map_err(|error| {
        JavaScriptHostError::InvalidConfiguration(format!(
            "cannot create JavaScript Host directory {}: {error}",
            path.display()
        ))
    })
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
    /// The generation owns this future and cancels it on failure or timeout.
    /// Handlers must yield cooperatively and must not detach work that can keep
    /// mutating state after the future is dropped. Cancellation does not roll
    /// back effects already committed by a handler.
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
    pub host_instance_id: JavaScriptHostInstanceId,
    pub host_generation: u64,
    /// Opaque acquisition lease, not the plugin's local resource ID. Never parse
    /// or reconstruct it; a reacquired resource receives a different lease.
    pub handle_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JavaScriptHostInstanceId(Uuid);

impl JavaScriptHostInstanceId {
    fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

#[async_trait]
pub trait ExtensionHostDemandPort: Send + Sync {
    async fn invoke_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, JavaScriptHostError>;

    async fn contribute_context_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        schema_ref: CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, JavaScriptHostError>;

    async fn acquire_resource_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        binding_id: ResourceBindingId,
        resource_kind: ResourceKind,
        parameters: StrictJsonValue,
    ) -> Result<JavaScriptResourceHandle, JavaScriptHostError>;

    async fn release_resource(
        &self,
        resource: &JavaScriptResourceHandle,
    ) -> Result<(), JavaScriptHostError>;
}

pub struct ExtensionHostSupervisor {
    instance_id: JavaScriptHostInstanceId,
    config: JavaScriptHostConfig,
    contract: PluginN1ContractManifest,
    host_kind: JavaScriptHostKind,
    services: Arc<dyn ExtensionHostServices>,
    state: Mutex<SupervisorState>,
    public_state: watch::Sender<JavaScriptHostState>,
}

struct SupervisorState {
    next_generation: u64,
    current: Option<GenerationHandle>,
    cleanup: Option<ChildProcessCleanup>,
}

#[derive(Clone)]
struct GenerationHandle {
    host_kind: JavaScriptHostKind,
    generation: u64,
    commands: mpsc::Sender<ActorCommand>,
    state: watch::Receiver<JavaScriptHostState>,
    mounts: Arc<RwLock<BTreeMap<PluginMountId, PluginMountRuntimeContext>>>,
    admission: Arc<RwLock<()>>,
}

impl ExtensionHostSupervisor {
    pub fn new(config: JavaScriptHostConfig) -> Result<Self, JavaScriptHostError> {
        Self::with_role_and_services(
            config,
            JavaScriptHostKind::SharedExtension,
            Arc::new(DenyExtensionHostServices),
        )
    }

    pub fn candidate_test(
        config: JavaScriptHostConfig,
    ) -> Result<Self, JavaScriptHostError> {
        Self::with_role_and_services(
            config,
            JavaScriptHostKind::CandidateTest,
            Arc::new(DenyExtensionHostServices),
        )
    }

    pub fn with_services(
        config: JavaScriptHostConfig,
        services: Arc<dyn ExtensionHostServices>,
    ) -> Result<Self, JavaScriptHostError> {
        Self::with_role_and_services(
            config,
            JavaScriptHostKind::SharedExtension,
            services,
        )
    }

    pub fn with_role_and_services(
        config: JavaScriptHostConfig,
        host_kind: JavaScriptHostKind,
        services: Arc<dyn ExtensionHostServices>,
    ) -> Result<Self, JavaScriptHostError> {
        if host_kind == JavaScriptHostKind::Build {
            return Err(JavaScriptHostError::InvalidConfiguration(
                "Build Host uses the dedicated authoring supervisor".into(),
            ));
        }
        config.validate_paths()?;
        let contract = PluginN1ContractManifest::canonical();
        contract
            .validate()
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        let (public_state, _) = watch::channel(JavaScriptHostState::Stopped);
        Ok(Self {
            instance_id: JavaScriptHostInstanceId::new(),
            config,
            contract,
            host_kind,
            services,
            state: Mutex::new(SupervisorState {
                next_generation: 0,
                current: None,
                cleanup: None,
            }),
            public_state,
        })
    }

    pub fn host_kind(&self) -> JavaScriptHostKind {
        self.host_kind
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

    /// Unload an idle Mount, releasing its resources and draining cleanup SDK
    /// calls before Ack. Concurrent work is rejected, not silently discarded.
    /// A failed disposal leaves unknown plugin state and fails the generation.
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
        let host_instance_id = self.instance_id;
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
                    host_instance_id,
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
            if self.instance_id != resource.host_instance_id
                || current.generation != resource.host_generation
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
        let mut state = self.state.lock().await;
        if let Some(current) = &state.current
            && matches!(current.state.borrow().clone(), JavaScriptHostState::Running { .. })
        {
            return current.commit_fence_for_mount(
                mount_id.clone(), self.config.limits.shutdown_timeout,
            ).await;
        }
        self.confirm_previous_cleanup(&mut state).await?;
        Ok(PluginHostCommitFence::NotResident)
    }

    /// Confirm that no generation is running and its process-tree cleanup has
    /// completed. This observes cleanup; it neither stops a live generation nor
    /// permanently closes admission. Replacement callers must exclude new
    /// demands separately (for example with the Runtime switch write lease).
    pub async fn confirm_stopped(&self) -> Result<(), JavaScriptHostError> {
        let mut state = self.state.lock().await;
        if let Some(current) = &state.current
            && matches!(current.state.borrow().clone(), JavaScriptHostState::Running { .. })
        {
            return Err(JavaScriptHostError::NotQuiescent { generation: current.generation });
        }
        self.confirm_previous_cleanup(&mut state).await
    }

    pub async fn stop_generation(
        &self,
        expected_generation: u64,
    ) -> Result<PluginHostCommitFence, JavaScriptHostError> {
        let handle = {
            let mut state = self.state.lock().await;
            let Some(current) = &state.current else {
                self.confirm_previous_cleanup(&mut state).await?;
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
        let deadline = Instant::now() + self.config.limits.shutdown_timeout;
        let _admission = tokio::time::timeout_at(deadline, handle.admission.write())
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?;
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
        let permit = tokio::time::timeout_at(deadline, handle.commands.reserve())
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?
            .map_err(|_| JavaScriptHostError::HostFailure {
                generation: expected_generation,
                reason: "Host command queue is closed".into(),
            })?;
        permit.send(ActorCommand::Stop { expected_generation, reply });
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

    async fn confirm_previous_cleanup(
        &self,
        state: &mut SupervisorState,
    ) -> Result<(), JavaScriptHostError> {
        let Some(receipt) = state.cleanup.clone() else { return Ok(()); };
        if !matches!(tokio::time::timeout(
            self.config.limits.shutdown_timeout, receipt.wait(),
        ).await, Ok(Ok(()))) {
            // Retain the same receipt on timeout, error or caller cancellation.
            // The managed process/relay owns termination; this gate only waits
            // for proof and never treats a public Failed state as that proof.
            return Err(JavaScriptHostError::HostFailure {
                generation: state.next_generation,
                reason: "previous Host process-tree cleanup is not confirmed".into(),
            });
        }
        state.cleanup = None;
        Ok(())
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

        self.confirm_previous_cleanup(&mut state).await?;

        state.next_generation = state
            .next_generation
            .checked_add(1)
            .ok_or_else(|| {
                JavaScriptHostError::InvalidConfiguration(
                    "Host generation counter overflowed".into(),
                )
            })?;
        let generation = state.next_generation;
        let result = spawn_generation(
            generation,
            &self.config,
            &self.contract,
            self.host_kind,
            Arc::clone(&self.services),
            self.public_state.clone(),
            &mut state.cleanup,
        )
        .await;
        let handle = result.map_err(|error| {
            self.public_state.send_replace(JavaScriptHostState::Failed {
                generation,
                reason: error.to_string(),
            });
            error
        })?;
        state.current = Some(handle.clone());
        Ok(handle)
    }
}

#[async_trait]
impl ExtensionHostDemandPort for ExtensionHostSupervisor {
    async fn invoke_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        ExtensionHostSupervisor::invoke_demand(
            self,
            mount,
            contribution,
            action_id,
            input,
        )
        .await
    }

    async fn contribute_context_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        schema_ref: CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        ExtensionHostSupervisor::contribute_context_demand(
            self,
            mount,
            contribution,
            schema_ref,
        )
        .await
    }

    async fn acquire_resource_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        binding_id: ResourceBindingId,
        resource_kind: ResourceKind,
        parameters: StrictJsonValue,
    ) -> Result<JavaScriptResourceHandle, JavaScriptHostError> {
        ExtensionHostSupervisor::acquire_resource_demand(
            self,
            mount,
            contribution,
            binding_id,
            resource_kind,
            parameters,
        )
        .await
    }

    async fn release_resource(
        &self,
        resource: &JavaScriptResourceHandle,
    ) -> Result<(), JavaScriptHostError> {
        ExtensionHostSupervisor::release_resource(self, resource).await
    }
}

impl GenerationHandle {
    async fn commit_fence_for_mount(
        &self,
        mount_id: PluginMountId,
        timeout: Duration,
    ) -> Result<PluginHostCommitFence, JavaScriptHostError> {
        let deadline = Instant::now() + timeout;
        // Drain submitters into the FIFO before querying the Actor's pending
        // and resident state. This is a point-in-time check, not a permanent
        // fence against demands admitted after this method returns.
        let _admission = tokio::time::timeout_at(deadline, self.admission.write())
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?;
        let (reply, response) = oneshot::channel();
        let permit = tokio::time::timeout_at(deadline, self.commands.reserve())
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?
            .map_err(|_| JavaScriptHostError::RequestChannelClosed)?;
        permit.send(ActorCommand::MountFence { mount_id, reply });
        tokio::time::timeout_at(deadline, response)
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?
            .unwrap_or(Err(JavaScriptHostError::RequestChannelClosed))
    }

    async fn request(
        &self,
        contract: &PluginN1ContractManifest,
        request: PluginHostRequest,
        module_path: Option<PathBuf>,
        timeout: Duration,
    ) -> Result<HostRequestHandle, JavaScriptHostError> {
        let deadline = Instant::now() + timeout;
        let _admission = tokio::time::timeout_at(deadline, self.admission.read())
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?;
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
            host_kind: self.host_kind,
            host_generation: self.generation,
            request_id: request_id.clone(),
            direction: request.direction(),
            request,
        };
        envelope
            .validate(contract)
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        let (reply, response) = oneshot::channel();
        let permit = tokio::time::timeout_at(deadline, self.commands.reserve())
            .await
            .map_err(|_| JavaScriptHostError::AdmissionTimeout)?
            .map_err(|_| JavaScriptHostError::HostFailure {
                generation: self.generation,
                reason: "Host command queue is closed".into(),
            })?;
        permit.send(ActorCommand::Submit {
            envelope: Box::new(envelope),
            module_path,
            deadline,
            reply,
        });
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
    MountFence {
        mount_id: PluginMountId,
        reply: oneshot::Sender<Result<PluginHostCommitFence, JavaScriptHostError>>,
    },
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

type HostServiceResult =
    Result<(PluginHostRequestEnvelope, PluginHostResponseBody, Instant), String>;

struct PendingRequest {
    envelope: PluginHostRequestEnvelope,
    deadline: Instant,
    reply: PendingReply,
}

enum PendingReply {
    Request {
        reply: oneshot::Sender<Result<PluginHostSuccess, JavaScriptHostError>>,
        coalesced: Vec<
            oneshot::Sender<Result<PluginHostSuccess, JavaScriptHostError>>,
        >,
    },
    Stop(
        oneshot::Sender<
            Result<PluginHostCommitFence, JavaScriptHostError>,
        >,
    ),
}

struct GenerationActor {
    host_kind: JavaScriptHostKind,
    generation: u64,
    process_id: u32,
    process: ManagedChildProcess,
    stdin: ChildStdin,
    outbound: OutboundQueue,
    reader_events: mpsc::Receiver<ReaderEvent>,
    commands: mpsc::Receiver<ActorCommand>,
    service_tasks: JoinSet<HostServiceResult>,
    service_requests: HashSet<CorrelationId>,
    pending: HashMap<CorrelationId, PendingRequest>,
    resource_mounts: HashMap<String, PluginMountId>,
    mounts: Arc<RwLock<BTreeMap<PluginMountId, PluginMountRuntimeContext>>>,
    services: Arc<dyn ExtensionHostServices>,
    contract: PluginN1ContractManifest,
    limits: JavaScriptHostLimits,
    state: watch::Sender<JavaScriptHostState>,
    public_state: watch::Sender<JavaScriptHostState>,
    reader_task: JoinHandle<()>,
    stderr_tasks: JoinSet<StderrSummary>,
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
    host_kind: JavaScriptHostKind,
    services: Arc<dyn ExtensionHostServices>,
    public_state: watch::Sender<JavaScriptHostState>,
    cleanup: &mut Option<ChildProcessCleanup>,
) -> Result<GenerationHandle, JavaScriptHostError> {
    verify_runtime(config).await?;
    let supported_methods = contract
        .host_method_sets
        .get(&host_kind)
        .cloned()
        .ok_or_else(|| {
            JavaScriptHostError::InvalidConfiguration(
                "selected JavaScript Host role is not in the N1 contract".into(),
            )
        })?;
    let bootstrap = serde_json::to_vec(&HostBootstrap {
        host_kind,
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
    // Record synchronously before the first post-spawn await. If startup is
    // cancelled, SupervisorState keeps this receipt while Drop owns cleanup.
    *cleanup = process.cleanup_receipt();
    let process_id = process.id().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node process did not expose a process id".into())
    })?;
    let stdin = process.stdin.take().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node stdin was not captured".into())
    })?;
    let stdout = process.stdout.take().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node stdout was not captured".into())
    })?;
    let stderr = process.stderr.take().ok_or_else(|| {
        JavaScriptHostError::Spawn("Node stderr was not captured".into())
    })?;
    // Start before Hello: a peer may await a stderr write before handshaking.
    // JoinSet aborts the collector if startup is cancelled; process ownership
    // stays with ManagedChildProcess and its existing Drop cleanup relay.
    let mut stderr_tasks = JoinSet::new();
    stderr_tasks.spawn(drain_stderr(stderr));

    let mut reader = BufReader::new(stdout);
    let handshake = async {
        let hello = tokio::time::timeout(
            config.limits.hello_timeout,
            read_json_line::<JavaScriptHostHello>(&mut reader, config.limits.max_frame_bytes),
        ).await.map_err(|_| "Hello timed out")?
            // serde errors can quote arbitrary peer input, including secrets.
            .map_err(|_| "Hello frame is invalid or exceeds the configured limit")?;
        hello.validate(contract).map_err(|_| "Hello violates the Host protocol contract")?;
        if hello.host_kind != host_kind
            || hello.host_generation != generation
            || hello.process_id != process_id
            || hello.runtime != config.runtime
        {
            return Err("Hello does not bind the selected Runtime, role, generation, and process");
        }
        Ok(())
    }.await;
    if let Err(reason) = handshake {
        let (cleanup_error, stderr) = cleanup_process(
            &mut process, &mut stderr_tasks, config.limits.shutdown_timeout,
        ).await;
        let cleanup = cleanup_error.map(|error| format!("; cleanup failed: {error}")).unwrap_or_default();
        return Err(JavaScriptHostError::HelloRejected(format!("{reason}{cleanup}{stderr}")));
    }

    let (reader_sender, reader_events) =
        mpsc::channel(config.limits.command_queue_capacity);
    let reader_task = tokio::spawn(read_frames(
        reader,
        reader_sender,
        config.limits.max_frame_bytes,
    ));
    let (command_sender, commands) =
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
        host_kind,
        generation,
        process_id,
        process,
        stdin,
        outbound: OutboundQueue::new(config.limits.command_queue_capacity, config.limits.max_frame_bytes),
        reader_events,
        commands,
        service_tasks: JoinSet::new(),
        service_requests: HashSet::new(),
        pending: HashMap::new(),
        resource_mounts: HashMap::new(),
        mounts: Arc::clone(&mounts),
        services,
        contract: contract.clone(),
        limits: config.limits.clone(),
        state,
        public_state,
        reader_task,
        stderr_tasks,
        accepting: true,
    };
    tokio::spawn(actor.run());

    Ok(GenerationHandle {
        host_kind,
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

async fn cleanup_process(
    process: &mut ManagedChildProcess,
    stderr: &mut JoinSet<StderrSummary>,
    timeout: Duration,
) -> (Option<String>, String) {
    let deadline = Instant::now() + timeout;
    let cleanup_error = match tokio::time::timeout_at(deadline, process.shutdown()).await {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error.to_string()),
        Err(_) => Some("process-tree shutdown timed out".into()),
    };
    (cleanup_error, finish_stderr(stderr, deadline).await)
}

async fn finish_stderr(tasks: &mut JoinSet<StderrSummary>, deadline: Instant) -> String {
    match tokio::time::timeout_at(deadline, tasks.join_next()).await {
        Ok(Some(Ok(summary))) if summary.bytes > 0 => format!(
            "; stderr contained {} bytes across {} lines", summary.bytes, summary.lines,
        ),
        Ok(Some(Err(_))) => "; stderr collector failed".into(),
        Err(_) => {
            tasks.shutdown().await;
            "; stderr drain timed out".into()
        }
        _ => String::new(),
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
        self.commands.close();
        self.fail_all(failure.clone());
        // The public terminal state and commit fence must not precede service
        // cancellation: a new generation may be admitted as soon as published.
        self.service_tasks.shutdown().await;
        let (cleanup_error, stderr) = cleanup_process(
            &mut self.process,
            &mut self.stderr_tasks,
            self.limits.shutdown_timeout,
        ).await;
        let fence = self.commit_fence();
        self.reader_task.abort();
        let _ = self.reader_task.await;

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
                    format!("{reason}; cleanup failed: {cleanup}{stderr}")
                } else {
                    format!("{reason}{stderr}")
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
            .min(self.limits.shutdown_timeout)
            .min(Duration::from_millis(50));
        let mut watchdog = tokio::time::interval(tick_period);
        watchdog.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                result = self.outbound.write_next(&mut self.stdin), if !self.outbound.is_empty() => {
                    match result {
                        Ok(Some(request_id)) => { self.service_requests.remove(&request_id); }
                        Ok(None) => {}
                        Err(reason) => return ActorExit::Failed(reason),
                    }
                }
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
                result = self.service_tasks.join_next(), if !self.service_tasks.is_empty() => {
                    if let Some(result) = result {
                        let result = result
                            // A handler's panic payload can contain credentials.
                            // Report the failure kind, not the arbitrary payload.
                            .map_err(|error| if error.is_panic() {
                                "Host service task panicked".to_owned()
                            } else {
                                "Host service task was cancelled".to_owned()
                            })
                            .and_then(|result| result);
                        match result {
                            Ok((request, response, deadline)) => {
                                if let Err(reason) = self.respond_to_service(request, response, deadline) {
                                    return ActorExit::Failed(reason);
                                }
                            }
                            Err(reason) => return ActorExit::Failed(reason),
                        }
                    }
                }
                _ = watchdog.tick() => {
                    if let Ok(Some(status)) = self.process.try_wait() {
                        return ActorExit::Failed(format!(
                            "JavaScript Host process exited with {status}"
                        ));
                    }
                    let now = Instant::now();
                    if self.outbound.has_expired(now) {
                        return ActorExit::Failed("Host IPC write timed out".into());
                    }
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
            ActorCommand::MountFence { mount_id, reply } => {
                let result = if !self.accepting {
                    Err(JavaScriptHostError::HostStopping { generation: self.generation })
                } else if self.mounts.read().await.contains_key(&mount_id)
                    || self.pending.values().any(|pending| {
                        self.request_mount_id(&pending.envelope.request) == Some(&mount_id)
                    })
                {
                    Err(JavaScriptHostError::NotQuiescent { generation: self.generation })
                } else {
                    Ok(PluginHostCommitFence::NotResident)
                };
                let _ = reply.send(result);
                None
            }
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
                if deadline <= Instant::now() {
                    let _ = reply.send(Err(JavaScriptHostError::AdmissionTimeout));
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
                if let Some(mount_id) = self.request_mount_id(&envelope.request) {
                    let unloading = self.pending.values().any(|pending| matches!(
                        &pending.envelope.request,
                        PluginHostRequest::MountUnload { target }
                            if &target.mount_id == mount_id
                    ));
                    let busy_unload = matches!(&envelope.request, PluginHostRequest::MountUnload { .. })
                        && self.pending.values().any(|pending| {
                            self.request_mount_id(&pending.envelope.request) == Some(mount_id)
                        });
                    if unloading || busy_unload {
                        let _ = reply.send(Err(JavaScriptHostError::NotQuiescent {
                            generation: self.generation,
                        }));
                        return None;
                    }
                }
                if let PluginHostRequest::MountLoad { context } = &envelope.request
                {
                    // The caller's resident check can race another load. Make
                    // idempotency authoritative here, where admission is serial.
                    if self.mounts.read().await.get(&context.target.mount_id)
                        == Some(context)
                    {
                        let _ = reply.send(Ok(PluginHostSuccess::Ack));
                        return None;
                    }
                }
                if !self.has_request_capacity(&envelope.request) {
                    let _ = reply.send(Err(JavaScriptHostError::QueueFull));
                    return None;
                }
                if let PluginHostRequest::MountLoad { context } = &envelope.request
                {
                    if let Some(pending) = self.pending.values_mut().find(|pending| {
                        pending.envelope.request == envelope.request
                    }) && let PendingReply::Request { coalesced, .. } = &mut pending.reply
                    {
                        // Identical first demands share activation and its result,
                        // without serializing independent Mounts or weakening
                        // identity checks.
                        coalesced.push(reply);
                        return None;
                    }
                    if self.mount_identity_is_reserved(context).await {
                        let _ = reply.send(Err(JavaScriptHostError::Contract(
                            "Mount ID and mount_handle_id must be unique within one Host generation"
                                .into(),
                        )));
                        return None;
                    }
                }
                let request_id = envelope.request_id.clone();
                let frame = PrivateRequestFrame {
                    kind: "request",
                    envelope: envelope.as_ref(),
                    module_path: module_path.as_deref(),
                };
                if let Err(error) = self.outbound.enqueue(&frame, deadline, None)
                {
                    let _ = reply.send(Err(error));
                    return None;
                }
                self.pending.insert(
                    request_id,
                    PendingRequest {
                        envelope: *envelope,
                        deadline,
                        reply: PendingReply::Request {
                            reply,
                            coalesced: Vec::new(),
                        },
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
                if !self.pending.is_empty() || !self.service_tasks.is_empty() || !self.outbound.is_empty() {
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
                    host_kind: self.host_kind,
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
                let deadline = Instant::now() + self.limits.shutdown_timeout;
                if let Err(error) = self.outbound.enqueue(&frame, deadline, None)
                {
                    let reason = error.to_string();
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
                        deadline,
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
                    .get(&response.request_id)
                    .ok_or("rejected unknown or late response")?;
                response
                    .validate_for(&pending.envelope)
                    .map_err(|_| "Host response violates the request contract")?;
                if matches!(&pending.envelope.request,
                    PluginHostRequest::MountUnload { .. } | PluginHostRequest::ResourceAcquire { .. })
                    && matches!(&response.response, PluginHostResponseBody::Failure(error)
                        if error.code.as_ref() == "PLUGIN_CLEANUP_FAILED")
                {
                    // Cleanup may have partially disposed the Mount. Fail closed
                    // through the normal generation teardown, including services.
                    return Err("Plugin cleanup failed".into());
                }
                // Only remove a validated response. On a protocol failure all
                // original/coalesced waiters remain available to fail_all.
                let pending = self.pending.remove(&response.request_id)
                    .expect("response was validated against this pending request");
                match pending.reply {
                    PendingReply::Request { reply, coalesced } => {
                        let result = response_result(
                            &pending.envelope.request_id,
                            response.response,
                        );
                        if let Ok(success) = &result {
                            self.apply_success(&pending.envelope, success).await;
                        }
                        for follower in coalesced {
                            let _ = follower.send(result.clone());
                        }
                        let _ = reply.send(result);
                        Ok(None)
                    }
                    PendingReply::Stop(reply) => match response_result(
                        &pending.envelope.request_id, response.response,
                    ) {
                        Ok(_) => Ok(Some(ActorExit::Stopped {
                            stop_reply: reply,
                        })),
                        Err(error) => {
                            let _ = reply.send(Err(error));
                            Err("HostShutdown was rejected by the peer".into())
                        }
                    },
                }
            }
            JavaScriptFrame::Request(request) => {
                if request.host_generation != self.generation || request.host_kind != self.host_kind {
                    return Err("Host service role or generation mismatch".into());
                }
                request
                    .validate(&self.contract)
                    .map_err(|_| "Host service request violates the protocol contract")?;
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
                    // Activation receives the SDK before Ack. Only the exact
                    // reserved MountLoad context may supply its early binding.
                    .or_else(|| self.pending.values().find_map(|pending| {
                        match &pending.envelope.request {
                            PluginHostRequest::MountLoad { context }
                                if context.mount_handle_id == mount_handle => Some(context.clone()),
                            _ => None,
                        }
                    }))
                    .ok_or_else(|| {
                        "JavaScript requested Host service for an unknown Mount handle"
                            .to_owned()
                    })?;
                // IDs remain reserved through response flush, not merely until
                // the handler future completes. Do not duplicate side effects
                // while the first response is still backpressured.
                if self.service_requests.contains(&request.request_id) {
                    return Err("duplicate outstanding Host service request".into());
                }
                if self.service_requests.len() >= self.limits.max_service_requests {
                    return Err("Host service request capacity exceeded".into());
                }
                self.service_requests.insert(request.request_id.clone());
                if !self.accepting {
                    self.respond_to_service(
                        request,
                        PluginHostResponseBody::Failure(PluginHostWireError {
                            code: CanonicalErrorCode::from(HOST_SERVICE_UNAVAILABLE),
                            message: "Host generation is stopping".into(),
                            retryable: false,
                        }),
                        Instant::now() + self.limits.shutdown_timeout,
                    )?;
                    return Ok(None);
                }
                let services = Arc::clone(&self.services);
                let deadline = Instant::now() + self.limits.request_timeout;
                self.service_tasks.spawn(async move {
                    let response = tokio::time::timeout_at(
                        deadline,
                        services.handle(BoundHostServiceRequest {
                            mount,
                            envelope: request.clone(),
                        }),
                    )
                    .await
                    .map_err(|_| "Host service timed out".to_owned())?;
                    Ok((request, response, deadline))
                });
                Ok(None)
            }
        }
    }

    fn respond_to_service(
        &mut self,
        request: PluginHostRequestEnvelope,
        response: PluginHostResponseBody,
        deadline: Instant,
    ) -> Result<(), String> {
        let envelope = PluginHostResponseEnvelope {
            protocol_version: VersionString::from(
                JAVASCRIPT_HOST_PROTOCOL_VERSION,
            ),
            host_kind: self.host_kind,
            host_generation: self.generation,
            request_id: request.request_id.clone(),
            response,
        };
        envelope
            .validate_for(&request)
            .map_err(|_| "Host service response violates the request contract")?;
        self.outbound.enqueue(&envelope, deadline, Some(request.request_id)).map_err(|error| error.to_string())
    }

    fn has_request_capacity(&self, request: &PluginHostRequest) -> bool {
        let cancellation = matches!(request, PluginHostRequest::RequestCancel { .. });
        let limit = if cancellation { 1 } else { self.limits.max_pending_requests };
        // Derive occupancy from owned replies instead of maintaining a second
        // counter across every completion/failure path. Coalescing saves wire
        // work, but each waiter still owns memory until completion.
        self.pending.values().filter(|pending| {
            matches!(pending.envelope.request, PluginHostRequest::RequestCancel { .. })
                == cancellation
        }).map(|pending| match &pending.reply {
            PendingReply::Request { coalesced, .. } => 1 + coalesced.len(),
            PendingReply::Stop(_) => 0,
        }).sum::<usize>() < limit
    }

    fn request_mount_id<'a>(&'a self, request: &'a PluginHostRequest) -> Option<&'a PluginMountId> {
        match request {
            PluginHostRequest::MountLoad { context } => Some(&context.target.mount_id),
            PluginHostRequest::MountUnload { target } => Some(&target.mount_id),
            PluginHostRequest::CapabilityInvoke { contribution, .. }
            | PluginHostRequest::ContextContribute { contribution, .. }
            | PluginHostRequest::ResourceAcquire { contribution, .. } => Some(&contribution.target.mount_id),
            PluginHostRequest::ResourceRelease { handle_id } => self.resource_mounts.get(handle_id),
            _ => None,
        }
    }

    async fn apply_success(
        &mut self,
        envelope: &PluginHostRequestEnvelope,
        success: &PluginHostSuccess,
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
                self.resource_mounts.retain(|_, owner| owner != &target.mount_id);
            }
            PluginHostRequest::ResourceAcquire { contribution, .. } => {
                if let PluginHostSuccess::ResourceAcquired { handle_id } = success {
                    self.resource_mounts.insert(handle_id.clone(), contribution.target.mount_id.clone());
                }
            }
            PluginHostRequest::ResourceRelease { handle_id } => {
                self.resource_mounts.remove(handle_id);
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
                PendingReply::Request { reply, coalesced } => {
                    for follower in coalesced {
                        let _ = follower.send(Err(error.clone()));
                    }
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
                ActorCommand::Stop { reply, .. } | ActorCommand::MountFence { reply, .. } => {
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

async fn drain_stderr(mut stderr: impl AsyncRead + Unpin) -> StderrSummary {
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

#[cfg(test)]
#[path = "admission_tests.rs"]
mod admission_tests;
#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod diagnostics_tests;
#[cfg(test)]
#[path = "cleanup_gate_tests.rs"]
mod cleanup_gate_tests;
