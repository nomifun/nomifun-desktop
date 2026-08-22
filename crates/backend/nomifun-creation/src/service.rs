//! [`CreationService`] — the generation task queue + state machine (contract §6
//! `service.rs`).
//!
//! The service owns the full lifecycle: `queued → running →
//! succeeded/failed/canceled`, a per-provider concurrency gate + a global cap,
//! synchronous and async (submit→poll) protocols, cancellation propagation,
//! boot reconciliation, and handing produced bytes to an [`AssetSink`]. Model
//! Media execution is delegated to the unified invocation layer
//! ([`nomifun_model_invoke::ModelInvokeService`]). Text creation is deliberately
//! delegated through [`CreationTextExecutor`] to the same Agent Chat engine used
//! by conversations; Chat is session/stream semantics, not a media protocol.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_common::{
    AppError, CreationTaskId, CreativeStudioNodeId, CreativeStudioProjectId,
    CreativeStudioWorkflowId, CreativeStudioWorkflowRunId, CreativeStudioWorkflowStepId,
    ProviderId, WorkshopAssetId, now_ms, validate_uuidv7,
};
#[cfg(test)]
use nomifun_common::generate_id;
use nomifun_db::{
    CreateCreativeTaskParams, CreationTaskPageCursorRef, CreationTaskRow, CreativeTaskOwnerRef,
    ICreationTaskRepository, ListStandaloneWorkbenchTasksParams,
    RetireStandaloneWorkbenchTasksParams, UpdateCreationTaskParams,
};
use nomifun_model_invoke::{
    ImageEditRequest, ImageGenRequest, InputAsset, InvokeErrorKind, JobHandle,
    MAX_ARTIFACT_BYTES, ModelInvokeService, ModelRef, ProducedAsset, ProducedData, TaskOutcome,
    TaskRequest, TaskResult, TtsRequest, VideoGenRequest,
};
use nomifun_net::egress::{SafeHttpClient, SafeHttpError, SafeHttpErrorKind, redacted_url};
use serde::Serialize;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::artifact::{reconcile_mime, validate_for_capability};
use crate::dto::CreationTask;
use crate::types::{
    CreationError, CreationInput, CreationInputKind, MediaCapability, StandaloneWorkbenchKind,
    TaskStatus,
};

/// Default per-provider in-flight cap (信号量).
const DEFAULT_PER_PROVIDER_LIMIT: usize = 3;
/// Default global in-flight cap across all providers.
const DEFAULT_GLOBAL_LIMIT: usize = 10;
/// Default poll interval for async submit→poll protocols.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(2500);
/// Default total budget for an async task before it is failed as `timeout`.
const DEFAULT_TASK_TIMEOUT: Duration = Duration::from_secs(600);
/// Timeout for fetching a URL-form artifact the adapter returned.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

/// The MIME stamped on produced text artifacts (the bridge keys its text-asset
/// special case off a `text/plain` prefix).
const TEXT_MIME: &str = "text/plain; charset=utf-8";
const DEFAULT_TEXT_MAX_TOKENS: u32 = 4096;

/// Complete one-shot text request handed from the creation state machine to
/// the application's Agent Chat execution bridge.
#[derive(Debug, Clone)]
pub struct CreationTextRequest {
    pub provider_id: String,
    pub model: String,
    pub system: String,
    pub prompt: String,
    pub max_tokens: u32,
}

/// Chat execution seam for Workshop text nodes.
///
/// Implementations must resolve the selected model's explicit Chat capability;
/// they must not infer a wire protocol from the provider platform.
#[async_trait]
pub trait CreationTextExecutor: Send + Sync {
    async fn complete(&self, request: CreationTextRequest) -> Result<String, CreationError>;
}

// ---------------------------------------------------------------------------
// Param helpers (ported verbatim from the retired adapters/mod.rs — the
// product-facing params contract stays owned by the creation engine).
// ---------------------------------------------------------------------------

/// The prompt string from the opaque params (`""` when absent).
fn param_prompt(params: &Value) -> String {
    params.get("prompt").and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

/// Maximum image batch size supported by the creation engine.
const MAX_IMAGE_OUTPUT_COUNT: u32 = 10;

/// Strict image batch count contract. Both the product-facing `count` name and
/// the OpenAI-compatible `n` alias are accepted. Omission means one image, but
/// a present value is never defaulted or clamped: malformed, zero, excessive,
/// or conflicting values fail the request.
fn param_count(params: &Value) -> Result<u32, CreationError> {
    let mut parsed = None;
    for field in ["count", "n"] {
        let Some(value) = params.get(field) else {
            continue;
        };
        let Some(value) = value.as_u64().filter(|value| *value > 0) else {
            return Err(CreationError::new(
                "invalid_params",
                format!("params.{field} must be a positive integer"),
            ));
        };
        if value > u64::from(MAX_IMAGE_OUTPUT_COUNT) {
            return Err(CreationError::new(
                "invalid_params",
                format!(
                    "params.{field} ({value}) exceeds the supported image output limit ({MAX_IMAGE_OUTPUT_COUNT})"
                ),
            ));
        }
        let value = value as u32;
        if parsed.is_some_and(|previous| previous != value) {
            return Err(CreationError::new(
                "invalid_params",
                "params.count and params.n must match when both are provided",
            ));
        }
        parsed = Some(value);
    }
    Ok(parsed.unwrap_or(1))
}

/// A `WxH` size string from `params.width`/`params.height`, or an explicit
/// `params.size` string, else `None`.
fn param_size(params: &Value) -> Option<String> {
    let w = params.get("width").and_then(|v| v.as_u64());
    let h = params.get("height").and_then(|v| v.as_u64());
    if let (Some(w), Some(h)) = (w, h) {
        return Some(format!("{w}x{h}"));
    }
    params
        .get("size")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

/// A non-empty string param (`params.{key}`), trimmed-empty treated as absent.
fn param_str(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `params.seconds` for video generation: absent → `None`; a JSON number or a
/// numeric string (both accepted by the retired openai_video adapter) →
/// `Some(u32)`; anything else present-but-unparseable is a typed local
/// `invalid_params` (the old code forwarded garbage to the provider; failing
/// locally is the honest replacement).
fn param_seconds(params: &Value) -> Result<Option<u32>, CreationError> {
    let Some(value) = params.get("seconds") else {
        return Ok(None);
    };
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .and_then(|v| u32::try_from(v).ok())
        .map(Some)
        .ok_or_else(|| {
            CreationError::new("invalid_params", "params.seconds must be a non-negative integer")
        })
}

fn param_text_max_tokens(params: &Value) -> Result<u32, CreationError> {
    let Some(value) = params.get("max_tokens") else {
        return Ok(DEFAULT_TEXT_MAX_TOKENS);
    };
    value
        .as_u64()
        .filter(|value| *value > 0)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            CreationError::new(
                "invalid_params",
                "params.max_tokens must be a positive 32-bit integer",
            )
        })
}

fn canonical_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical_json(value)))
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

/// Provider adapters receive only protocol-specific extras. Canonical fields
/// already projected into typed request members must not be sent a second time
/// through multipart/scalar transports, and UI-local presentation keys must
/// never leak to providers.
fn request_extra(params: &Value, consumed: &[&str]) -> Value {
    let Some(object) = params.as_object() else {
        return params.clone();
    };
    let mut extra = object.clone();
    for key in consumed {
        extra.remove(*key);
    }
    for key in [
        "canvasOperation",
        "sourceNodeId",
        "sourceAssetId",
        "markedReferenceAssetId",
        "userPrompt",
        "referenceWidth",
        "referenceHeight",
        "nomifunStandaloneWorkbench",
    ] {
        extra.remove(key);
    }
    Value::Object(extra)
}

/// Map a creation capability + opaque params + loaded inputs onto the invoke
/// layer's typed [`TaskRequest`]. The full params object rides along as
/// `extra` so protocol-specific knobs (`max_tokens`, `steps`, …) stay
/// reachable by adapters that understand them.
fn cap_to_task_request(
    capability: MediaCapability,
    params: &Value,
    inputs: Vec<InputAsset>,
) -> Result<TaskRequest, CreationError> {
    Ok(match capability {
        MediaCapability::T2i => TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: param_prompt(params),
            count: param_count(params)?,
            size: param_size(params),
            quality: param_str(params, "quality"),
            extra: request_extra(
                params,
                &["prompt", "count", "n", "width", "height", "size", "quality", "interface_mode"],
            ),
        }),
        MediaCapability::I2i | MediaCapability::Inpaint => TaskRequest::ImageEdit(ImageEditRequest {
            prompt: param_prompt(params),
            count: param_count(params)?,
            size: param_size(params),
            inputs,
            extra: request_extra(
                params,
                &["prompt", "count", "n", "width", "height", "size", "interface_mode"],
            ),
        }),
        MediaCapability::T2v | MediaCapability::I2v => TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: param_prompt(params),
            seconds: param_seconds(params)?,
            size: param_size(params),
            inputs,
            extra: request_extra(
                params,
                &["prompt", "seconds", "width", "height", "size", "resolution", "aspect"],
            ),
        }),
        MediaCapability::V2v => {
            return Err(CreationError::new(
                "unsupported_capability",
                "video-to-video (v2v) is not supported by any protocol adapter",
            ));
        }
        MediaCapability::Tts => TaskRequest::SpeechSynthesis(TtsRequest {
            text: param_prompt(params),
            voice: param_str(params, "voice"),
            format: param_str(params, "format"),
            extra: request_extra(params, &["prompt", "text", "voice", "format"]),
        }),
        MediaCapability::Text => {
            return Err(CreationError::config(
                "text creation must execute through the Agent Chat executor",
            ));
        }
    })
}

/// Decode the complete, protocol-owned handle stored for an async task.
/// Invalid data is rejected instead of guessing an adapter from capability.
fn parse_job_handle(raw: &str) -> Result<JobHandle, CreationError> {
    let handle = serde_json::from_str::<JobHandle>(raw).map_err(|error| {
        CreationError::config(format!("persisted remote task handle is invalid: {error}"))
    })?;
    if handle.adapter_id.trim().is_empty() || handle.remote_id.trim().is_empty() {
        return Err(CreationError::config(
            "persisted remote task handle must contain adapter_id, remote_id, and config_revision",
        ));
    }
    Ok(handle)
}

/// Resolve the minimum artifact count promised by a task. Image quantities are
/// part of the public request contract; every other currently-supported media
/// capability produces one artifact per task.
fn required_artifact_count(
    capability: MediaCapability,
    params: &Value,
) -> Result<usize, CreationError> {
    if matches!(
        capability,
        MediaCapability::T2i | MediaCapability::I2i | MediaCapability::Inpaint
    ) {
        Ok(param_count(params)? as usize)
    } else {
        Ok(1)
    }
}

/// A generation request accepted by [`CreationService::create_creative_task`].
pub struct NewCreationTask {
    pub provider_id: String,
    pub model: String,
    /// Wire capability code (`t2i|i2i|…`).
    pub capability: String,
    /// Opaque parameter map (prompt/size/quality/…).
    pub params: Value,
    pub inputs: Vec<CreationInput>,
}

pub const DEFAULT_STANDALONE_TASK_PAGE_LIMIT: usize = 30;
pub const MAX_STANDALONE_TASK_PAGE_LIMIT: usize = 100;

#[derive(Debug, Clone)]
pub struct StandaloneWorkbenchTaskPage {
    pub items: Vec<CreationTask>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StandaloneTaskPageCursor {
    submitted_at: i64,
    creation_task_id: String,
}

fn parse_standalone_task_cursor(raw: &str) -> Result<StandaloneTaskPageCursor, AppError> {
    let (timestamp, task_id) = raw.split_once(':').ok_or_else(|| {
        AppError::BadRequest(
            "cursor must be '<submitted_at>:<creation_task_uuidv7>'".into(),
        )
    })?;
    if timestamp.is_empty() || task_id.is_empty() || task_id.contains(':') {
        return Err(AppError::BadRequest(
            "cursor must contain exactly one timestamp/task separator".into(),
        ));
    }
    let submitted_at = timestamp.parse::<i64>().map_err(|_| {
        AppError::BadRequest("cursor submitted_at must be a canonical non-negative integer".into())
    })?;
    if submitted_at < 0 || submitted_at.to_string() != timestamp {
        return Err(AppError::BadRequest(
            "cursor submitted_at must be a canonical non-negative integer".into(),
        ));
    }
    let creation_task_id = CreationTaskId::parse(task_id)
        .map_err(|error| AppError::BadRequest(format!("invalid cursor task id: {error}")))?
        .into_string();
    Ok(StandaloneTaskPageCursor {
        submitted_at,
        creation_task_id,
    })
}

fn encode_standalone_task_cursor(task: &CreationTaskRow) -> String {
    format!("{}:{}", task.submitted_at, task.creation_task_id)
}

/// Canonical Creative Studio task owner. The API accepts this tagged union and
/// persists exactly one branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreativeTaskOwner {
    CanvasNode {
        project_id: String,
        node_id: String,
    },
    StandaloneWorkbench {
        project_id: String,
        workbench_kind: StandaloneWorkbenchKind,
    },
    WorkflowStep {
        workflow_id: String,
        workflow_run_id: String,
        workflow_step_id: String,
    },
}

impl CreativeTaskOwner {
    fn normalize(self) -> Result<Self, AppError> {
        match self {
            Self::CanvasNode {
                project_id,
                node_id,
            } => Ok(Self::CanvasNode {
                project_id: CreativeStudioProjectId::parse(project_id)
                    .map_err(|error| AppError::BadRequest(format!("invalid project_id: {error}")))?
                    .into_string(),
                node_id: CreativeStudioNodeId::parse(node_id)
                    .map_err(|error| AppError::BadRequest(format!("invalid node_id: {error}")))?
                    .into_string(),
            }),
            Self::StandaloneWorkbench {
                project_id,
                workbench_kind,
            } => Ok(Self::StandaloneWorkbench {
                project_id: CreativeStudioProjectId::parse(project_id)
                    .map_err(|error| AppError::BadRequest(format!("invalid project_id: {error}")))?
                    .into_string(),
                workbench_kind,
            }),
            Self::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            } => Ok(Self::WorkflowStep {
                workflow_id: CreativeStudioWorkflowId::parse(workflow_id)
                    .map_err(|error| AppError::BadRequest(format!("invalid workflow_id: {error}")))?
                    .into_string(),
                workflow_run_id: CreativeStudioWorkflowRunId::parse(workflow_run_id)
                    .map_err(|error| {
                        AppError::BadRequest(format!("invalid workflow_run_id: {error}"))
                    })?
                    .into_string(),
                workflow_step_id: CreativeStudioWorkflowStepId::parse(workflow_step_id)
                    .map_err(|error| {
                        AppError::BadRequest(format!("invalid workflow_step_id: {error}"))
                    })?
                    .into_string(),
            }),
        }
    }

    fn as_repository_owner(&self) -> CreativeTaskOwnerRef<'_> {
        match self {
            Self::CanvasNode {
                project_id,
                node_id,
            } => CreativeTaskOwnerRef::CanvasNode {
                project_id,
                node_id,
            },
            Self::StandaloneWorkbench {
                project_id,
                workbench_kind,
            } => CreativeTaskOwnerRef::StandaloneWorkbench {
                project_id,
                workbench_kind: workbench_kind.as_str(),
            },
            Self::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            } => CreativeTaskOwnerRef::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            },
        }
    }
}

struct PreparedCreationTask {
    provider_id: String,
    model: String,
    capability: MediaCapability,
    params: Value,
    params_json: String,
    required_artifact_count: usize,
    inputs: Vec<CreationInput>,
}

#[derive(Serialize)]
struct CanonicalCreativeTaskRequest<'a> {
    owner: &'a CreativeTaskOwner,
    provider_id: &'a str,
    model: &'a str,
    capability: &'a str,
    params: &'a Value,
    inputs: &'a [CreationInput],
}

impl PreparedCreationTask {
    fn into_worker_job(
        self,
        creation_task_id: String,
        owner: CreativeTaskOwner,
        submitted_at: i64,
    ) -> WorkerJob {
        let (
            project_id,
            workbench_kind,
            workflow_id,
            workflow_run_id,
            workflow_step_id,
            node_id,
        ) = match owner {
            CreativeTaskOwner::CanvasNode {
                project_id,
                node_id,
            } => (Some(project_id), None, None, None, None, Some(node_id)),
            CreativeTaskOwner::StandaloneWorkbench {
                project_id,
                workbench_kind,
            } => (
                Some(project_id),
                Some(workbench_kind),
                None,
                None,
                None,
                None,
            ),
            CreativeTaskOwner::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            } => (
                None,
                None,
                Some(workflow_id),
                Some(workflow_run_id),
                Some(workflow_step_id),
                None,
            ),
        };
        WorkerJob {
            creation_task_id,
            project_id,
            workbench_kind,
            workflow_id,
            workflow_run_id,
            workflow_step_id,
            node_id,
            provider_id: self.provider_id,
            model: self.model,
            capability: self.capability,
            params: self.params,
            required_artifact_count: self.required_artifact_count,
            inputs: self.inputs,
            submitted_at,
            remote_task_id: None,
        }
    }
}

/// A produced artifact ready for persistence: resolved bytes (URL artifacts are
/// fetched by the engine first) + MIME + provenance.
pub struct PersistAsset {
    pub bytes: Vec<u8>,
    pub mime: String,
    /// Whether the produced asset appears in the asset library. Generated
    /// products default to `true` (see [`CreationService::persist_assets`]).
    pub in_library: bool,
    /// Canonical provenance, including exactly one project/workflow owner
    /// branch plus provider/model/task metadata.
    pub origin: Value,
}

/// An input asset loaded to bytes (returned by [`AssetSource`]).
pub struct LoadedAsset {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Durable task→artifact manifest used at the sink trust boundary. `committed`
/// means the task row claims `succeeded`; the sink still verifies that every
/// claimed id exists, belongs to the task, and is locatable.
#[derive(Debug, Clone)]
pub struct TaskArtifactManifest {
    pub creation_task_id: String,
    pub committed: bool,
    pub asset_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskArtifactIssue {
    pub creation_task_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskArtifactCleanupFailure {
    pub creation_task_id: Option<String>,
    pub asset_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct TaskArtifactReconcileReport {
    pub removed_assets: usize,
    pub invalid_committed_tasks: Vec<TaskArtifactIssue>,
    pub cleanup_failures: Vec<TaskArtifactCleanupFailure>,
}

/// Where produced artifacts are persisted — implemented by the app over
/// `nomifun-workshop` (registers each result with a bare canonical UUIDv7
/// `asset_id`), so this crate never depends on `nomifun-workshop` (no
/// dependency cycle).
#[async_trait]
pub trait AssetSink: Send + Sync {
    /// Persist one produced artifact and return its new bare UUIDv7 asset ID.
    ///
    /// Returning `Err` MUST leave no newly-created asset behind. Once this
    /// method returns `Ok`, ownership remains provisional until the creation
    /// task's terminal `succeeded` state is committed.
    async fn persist(&self, asset: PersistAsset) -> Result<String, CreationError>;

    /// Remove assets provisionally persisted for a batch that did not commit.
    ///
    /// Implementations MUST be idempotent: an already-absent id is success.
    /// The service only passes ids returned by this sink for the current task.
    async fn rollback(&self, asset_ids: &[String]) -> Result<(), CreationError>;

    /// Verify committed task manifests without mutating assets. Implementations
    /// should batch this operation so list queries require one asset scan.
    async fn verify_task_artifacts(
        &self,
        committed_tasks: &[TaskArtifactManifest],
    ) -> Result<Vec<TaskArtifactIssue>, CreationError>;

    /// Boot-time complete-inventory reconciliation. Implementations scan their
    /// asset inventory once, preserve only valid assets claimed by succeeded
    /// tasks, and remove task-origin assets for every non-succeeded, missing,
    /// unknown-status, or otherwise invalid task.
    async fn reconcile_task_artifacts(
        &self,
        all_tasks: &[TaskArtifactManifest],
    ) -> Result<TaskArtifactReconcileReport, CreationError>;
}

/// Where task input assets are read from — the mirror of [`AssetSink`], also
/// implemented by the app over `nomifun-workshop`.
#[async_trait]
pub trait AssetSource: Send + Sync {
    /// Load an asset's bytes + MIME by its bare UUIDv7 `asset_id`.
    async fn load(&self, asset_id: &str) -> Result<LoadedAsset, CreationError>;
}

/// The persisted fields a worker needs to run (or resume) one task.
struct WorkerJob {
    creation_task_id: String,
    project_id: Option<String>,
    workbench_kind: Option<StandaloneWorkbenchKind>,
    workflow_id: Option<String>,
    workflow_run_id: Option<String>,
    workflow_step_id: Option<String>,
    node_id: Option<String>,
    provider_id: String,
    model: String,
    capability: MediaCapability,
    params: Value,
    /// Validated once when the task is accepted (or defensively revalidated
    /// when a durable remote task is resumed). Execution must not reinterpret
    /// or silently normalize the quantity later.
    required_artifact_count: usize,
    inputs: Vec<CreationInput>,
    submitted_at: i64,
    /// Present only on a boot resume (skip submit, poll this remote job).
    /// Carries the raw persisted column value: serialized [`JobHandle`] JSON.
    remote_task_id: Option<String>,
}

/// The result of running one task through an adapter.
enum ExecOutcome {
    Succeeded(Vec<String>),
    Failed(CreationError),
    /// Cancelled mid-flight — the terminal `canceled` status was already written
    /// by [`CreationService::cancel_task`], so the worker must not overwrite it.
    Canceled,
}

pub struct CreationService {
    repo: Arc<dyn ICreationTaskRepository>,
    /// The unified model invocation layer (`None` in the bare skeleton —
    /// tasks then fail `config`).
    invoke: Option<Arc<ModelInvokeService>>,
    text_executor: Option<Arc<dyn CreationTextExecutor>>,
    artifact_downloader: SafeHttpClient,
    asset_source: Option<Arc<dyn AssetSource>>,
    asset_sink: Option<Arc<dyn AssetSink>>,
    global_sem: Arc<Semaphore>,
    per_provider_limit: usize,
    provider_sems: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// Live task id → its cancellation token (present while queued/running).
    inflight: Mutex<HashMap<String, CancellationToken>>,
    poll_interval: Duration,
    task_timeout: Duration,
    #[cfg(test)]
    test_project_id: Option<String>,
}

/// Builder for [`CreationService`] (the app wires the invoke layer + sink).
pub struct CreationServiceBuilder {
    repo: Arc<dyn ICreationTaskRepository>,
    invoke: Option<Arc<ModelInvokeService>>,
    text_executor: Option<Arc<dyn CreationTextExecutor>>,
    artifact_downloader: Option<SafeHttpClient>,
    asset_source: Option<Arc<dyn AssetSource>>,
    asset_sink: Option<Arc<dyn AssetSink>>,
    per_provider_limit: usize,
    global_limit: usize,
    poll_interval: Duration,
    task_timeout: Duration,
    #[cfg(test)]
    test_project_id: Option<String>,
}

impl CreationServiceBuilder {
    /// Wire the unified model invocation service (provider/model resolution +
    /// protocol adapters live there).
    pub fn with_invoke(mut self, invoke: Arc<ModelInvokeService>) -> Self {
        self.invoke = Some(invoke);
        self
    }

    /// Wire Workshop text nodes to the Agent Chat engine.
    pub fn with_text_executor(mut self, executor: Arc<dyn CreationTextExecutor>) -> Self {
        self.text_executor = Some(executor);
        self
    }

    #[cfg(test)]
    fn with_artifact_downloader_for_tests(mut self, downloader: SafeHttpClient) -> Self {
        self.artifact_downloader = Some(downloader);
        self
    }

    #[cfg(test)]
    fn with_test_project_id(mut self, project_id: String) -> Self {
        self.test_project_id = Some(project_id);
        self
    }

    pub fn with_asset_source(mut self, source: Arc<dyn AssetSource>) -> Self {
        self.asset_source = Some(source);
        self
    }

    pub fn with_asset_sink(mut self, sink: Arc<dyn AssetSink>) -> Self {
        self.asset_sink = Some(sink);
        self
    }

    /// Override the poll interval (async protocols) — primarily for tests.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Override the async task timeout — primarily for tests.
    pub fn with_task_timeout(mut self, timeout: Duration) -> Self {
        self.task_timeout = timeout;
        self
    }

    pub fn build(self) -> Arc<CreationService> {
        Arc::new(CreationService {
            repo: self.repo,
            invoke: self.invoke,
            text_executor: self.text_executor,
            artifact_downloader: self.artifact_downloader.unwrap_or_else(|| {
                SafeHttpClient::new(DOWNLOAD_TIMEOUT, MAX_ARTIFACT_BYTES as usize)
                    .user_agent("NomiFun-Creation/1.0")
            }),
            asset_source: self.asset_source,
            asset_sink: self.asset_sink,
            global_sem: Arc::new(Semaphore::new(self.global_limit)),
            per_provider_limit: self.per_provider_limit,
            provider_sems: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            poll_interval: self.poll_interval,
            task_timeout: self.task_timeout,
            #[cfg(test)]
            test_project_id: self.test_project_id,
        })
    }
}

impl CreationService {
    /// Start a builder over the task repo (invoke layer/sink layered on).
    pub fn builder(repo: Arc<dyn ICreationTaskRepository>) -> CreationServiceBuilder {
        CreationServiceBuilder {
            repo,
            invoke: None,
            text_executor: None,
            artifact_downloader: None,
            asset_source: None,
            asset_sink: None,
            per_provider_limit: DEFAULT_PER_PROVIDER_LIMIT,
            global_limit: DEFAULT_GLOBAL_LIMIT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            task_timeout: DEFAULT_TASK_TIMEOUT,
            #[cfg(test)]
            test_project_id: None,
        }
    }

    /// Build a bare service over just the task repo (no invoke layer — tasks
    /// created against it fail `config`). Full wiring uses
    /// [`CreationService::builder`].
    pub fn new(repo: Arc<dyn ICreationTaskRepository>) -> Arc<Self> {
        Self::builder(repo).build()
    }

    // -----------------------------------------------------------------------
    // Public surface (routes)
    // -----------------------------------------------------------------------

    async fn prepare_task(
        &self,
        req: NewCreationTask,
    ) -> Result<PreparedCreationTask, AppError> {
        let capability = MediaCapability::parse(&req.capability).ok_or_else(|| {
            AppError::BadRequest(format!(
                "unknown capability '{}' (expected t2i|i2i|inpaint|t2v|i2v|v2v|tts|text)",
                req.capability
            ))
        })?;
        let required_artifact_count = required_artifact_count(capability, &req.params)
            .map_err(|error| AppError::BadRequest(error.message))?;
        let provider_id = ProviderId::parse(req.provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?
            .into_string();
        if req.model.trim().is_empty() {
            return Err(AppError::BadRequest("model must not be empty".into()));
        }
        let inputs = req
            .inputs
            .into_iter()
            .map(|input| {
                let asset_id = WorkshopAssetId::parse(input.asset_id)
                    .map_err(|error| AppError::BadRequest(format!("invalid input asset_id: {error}")))?
                    .into_string();
                Ok(CreationInput {
                    asset_id,
                    kind: input.kind,
                    role: input.role,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let params = canonical_json(req.params);
        let params_json = serde_json::to_string(&params)
            .map_err(|e| AppError::BadRequest(format!("invalid params json: {e}")))?;

        Ok(PreparedCreationTask {
            provider_id,
            model: req.model,
            capability,
            params,
            params_json,
            required_artifact_count,
            inputs,
        })
    }

    /// Canonical Creative Studio create. The UUIDv7 Idempotency-Key is also the
    /// task business id, making response-loss retries durable across reloads
    /// and process restarts. Only the transaction's `inserted` result may spawn
    /// a worker.
    pub async fn create_creative_task(
        self: &Arc<Self>,
        owner: CreativeTaskOwner,
        idempotency_key: String,
        req: NewCreationTask,
    ) -> Result<CreationTask, AppError> {
        let creation_task_id = CreationTaskId::parse(idempotency_key)
            .map_err(|error| AppError::BadRequest(format!("invalid Idempotency-Key: {error}")))?
            .into_string();
        let owner = owner.normalize()?;
        if !req.params.is_object() {
            return Err(AppError::BadRequest(
                "Creative Studio task params must be a JSON object".into(),
            ));
        }
        // The atomic repository operation validates current project/provider
        // state only for a brand-new key. Skipping the eager provider lookup
        // here keeps an exact historical replay readable after retirement.
        let prepared = self.prepare_task(req).await?;
        if let CreativeTaskOwner::StandaloneWorkbench { workbench_kind, .. } = &owner
            && !workbench_kind.accepts_capability(prepared.capability)
        {
            return Err(AppError::BadRequest(format!(
                "standalone {} workbench cannot own capability {}",
                workbench_kind.as_str(),
                prepared.capability.as_str()
            )));
        }
        if prepared.model.trim() != prepared.model {
            return Err(AppError::BadRequest(
                "Creative Studio model must be already normalized".into(),
            ));
        }
        if let Some(input) = prepared.inputs.iter().find(|input| {
            !matches!(
                input.role.as_str(),
                "reference" | "mask" | "first_frame" | "last_frame" | "video" | "audio"
            )
        }) {
            return Err(AppError::BadRequest(format!(
                "unsupported Creative Studio input role '{}'",
                input.role
            )));
        }
        if let Some(input) = prepared.inputs.iter().find(|input| match input.role.as_str() {
            "mask" | "first_frame" | "last_frame" => input.kind != CreationInputKind::Image,
            "video" => input.kind != CreationInputKind::Video,
            "audio" => input.kind != CreationInputKind::Audio,
            "reference" => false,
            _ => false,
        }) {
            return Err(AppError::BadRequest(format!(
                "Creative Studio input role '{}' is incompatible with kind '{}'",
                input.role,
                input.kind.as_str()
            )));
        }
        let input_bindings = serde_json::to_string(&prepared.inputs)
            .map_err(|error| AppError::BadRequest(format!("invalid input bindings: {error}")))?;
        let request_fingerprint = serde_json::to_string(&CanonicalCreativeTaskRequest {
            owner: &owner,
            provider_id: &prepared.provider_id,
            model: &prepared.model,
            capability: prepared.capability.as_str(),
            params: &prepared.params,
            inputs: &prepared.inputs,
        })
        .map_err(|error| AppError::BadRequest(format!("invalid canonical creation request: {error}")))?;
        let now = now_ms();
        let outcome = self
            .repo
            .get_or_create_creative_task(CreateCreativeTaskParams {
                creation_task_id: &creation_task_id,
                owner: owner.as_repository_owner(),
                provider_id: &prepared.provider_id,
                model: &prepared.model,
                capability: prepared.capability.as_str(),
                params: &prepared.params_json,
                input_bindings: &input_bindings,
                request_fingerprint: &request_fingerprint,
                status: TaskStatus::Queued.as_str(),
                submitted_at: now,
            })
            .await?;
        if outcome.inserted {
            self.spawn(prepared.into_worker_job(creation_task_id, owner, now));
            return outcome.row.try_into();
        }
        let mut rows = self.audit_rows_for_output(vec![outcome.row]).await?;
        rows.pop()
            .expect("one idempotent task remains after artifact audit")
            .try_into()
    }

    #[cfg(test)]
    async fn create_test_task(
        self: &Arc<Self>,
        req: NewCreationTask,
    ) -> Result<CreationTask, AppError> {
        let project_id = self
            .test_project_id
            .clone()
            .expect("test service must be configured with a canonical project");
        self.create_creative_task(
            CreativeTaskOwner::CanvasNode {
                project_id,
                node_id: CreativeStudioNodeId::new().into_string(),
            },
            CreationTaskId::new().into_string(),
            req,
        )
        .await
    }

    pub async fn get_task(&self, creation_task_id: &str) -> Result<CreationTask, AppError> {
        validate_uuidv7(creation_task_id)
            .map_err(|error| AppError::BadRequest(format!("invalid creation_task_id: {error}")))?;
        let row = self
            .repo
            .get_task(creation_task_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("creation task {creation_task_id} not found")))?;
        let mut rows = self.audit_rows_for_output(vec![row]).await?;
        rows.pop().expect("one task row remains after artifact audit").try_into()
    }

    pub async fn list_standalone_workbench_tasks(
        &self,
        project_id: &str,
        workbench_kind: StandaloneWorkbenchKind,
        active_only: bool,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<StandaloneWorkbenchTaskPage, AppError> {
        let project_id = CreativeStudioProjectId::parse(project_id)
            .map_err(|error| AppError::BadRequest(format!("invalid project_id: {error}")))?
            .into_string();
        let limit = limit.unwrap_or(DEFAULT_STANDALONE_TASK_PAGE_LIMIT);
        if !(1..=MAX_STANDALONE_TASK_PAGE_LIMIT).contains(&limit) {
            return Err(AppError::BadRequest(format!(
                "limit must be between 1 and {MAX_STANDALONE_TASK_PAGE_LIMIT}"
            )));
        }
        let cursor = cursor.map(parse_standalone_task_cursor).transpose()?;
        let mut rows = self
            .repo
            .list_standalone_workbench_tasks_page(ListStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: workbench_kind.as_str(),
                active_only,
                before: cursor.as_ref().map(|cursor| CreationTaskPageCursorRef {
                    submitted_at: cursor.submitted_at,
                    creation_task_id: &cursor.creation_task_id,
                }),
                limit,
            })
            .await?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        for row in &rows {
            let exact_owner = row.project_id.as_deref() == Some(project_id.as_str())
                && row.workbench_kind.as_deref() == Some(workbench_kind.as_str())
                && row.node_id.is_none()
                && row.workflow_id.is_none()
                && row.workflow_run_id.is_none()
                && row.workflow_step_id.is_none();
            let capability_matches = MediaCapability::parse(&row.capability)
                .is_some_and(|capability| workbench_kind.accepts_capability(capability));
            if !exact_owner || !capability_matches {
                return Err(AppError::Internal(format!(
                    "standalone task {} escaped its exact owner/capability scope",
                    row.creation_task_id
                )));
            }
        }
        let next_cursor = has_more && !rows.is_empty();
        let encoded_cursor = next_cursor
            .then(|| rows.last().map(encode_standalone_task_cursor))
            .flatten();
        let rows = self.audit_rows_for_output(rows).await?;
        let items = rows
            .into_iter()
            .map(CreationTask::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StandaloneWorkbenchTaskPage {
            items,
            next_cursor: encoded_cursor,
        })
    }

    pub async fn retire_standalone_workbench_tasks(
        &self,
        project_id: &str,
        workbench_kind: StandaloneWorkbenchKind,
        task_ids: &[String],
    ) -> Result<Vec<String>, AppError> {
        let project_id = CreativeStudioProjectId::parse(project_id)
            .map_err(|error| AppError::BadRequest(format!("invalid project_id: {error}")))?
            .into_string();
        if task_ids.is_empty() || task_ids.len() > 100 {
            return Err(AppError::BadRequest(
                "retire requires between 1 and 100 task_ids".into(),
            ));
        }
        let mut seen = HashSet::with_capacity(task_ids.len());
        for task_id in task_ids {
            CreationTaskId::parse(task_id).map_err(|error| {
                AppError::BadRequest(format!("invalid retire task id {task_id:?}: {error}"))
            })?;
            if !seen.insert(task_id.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "retire task_ids contains duplicate {task_id}"
                )));
            }
        }

        let mut rows = Vec::with_capacity(task_ids.len());
        for task_id in task_ids {
            let row = self
                .repo
                .get_task(task_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("creation task {task_id} not found")))?;
            let exact_owner = row.project_id.as_deref() == Some(project_id.as_str())
                && row.workbench_kind.as_deref() == Some(workbench_kind.as_str())
                && row.node_id.is_none()
                && row.workflow_id.is_none()
                && row.workflow_run_id.is_none()
                && row.workflow_step_id.is_none();
            let capability_matches = MediaCapability::parse(&row.capability)
                .is_some_and(|capability| workbench_kind.accepts_capability(capability));
            if !exact_owner || !capability_matches {
                return Err(AppError::Conflict(format!(
                    "creation task {task_id} does not belong to the requested standalone workbench owner"
                )));
            }
            if matches!(row.status.as_str(), "queued" | "running") {
                return Err(AppError::Conflict(format!(
                    "live creation task {task_id} cannot be retired"
                )));
            }
            if !matches!(row.status.as_str(), "failed" | "canceled" | "succeeded") {
                return Err(AppError::Conflict(format!(
                    "creation task {task_id} is not in a supported terminal state"
                )));
            }
            rows.push(row);
        }
        let deleted_at = rows
            .iter()
            .map(|row| row.submitted_at)
            .max()
            .unwrap_or_default()
            .max(now_ms());
        let rows = self.audit_rows_for_output(rows).await?;
        for row in rows {
            CreationTask::try_from(row)?;
        }
        let retired = self
            .repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: workbench_kind.as_str(),
                task_ids,
                deleted_at,
            })
            .await?;
        if retired.len() != task_ids.len() {
            return Err(AppError::Internal(
                "retirement returned an incomplete task batch".into(),
            ));
        }
        for (expected, row) in task_ids.iter().zip(&retired) {
            if row.creation_task_id != *expected || row.deleted_at.is_none() {
                return Err(AppError::Internal(
                    "retirement returned a reordered or untombstoned task".into(),
                ));
            }
        }
        Ok(task_ids.to_vec())
    }

    /// Cancel a task. Terminal tasks are returned unchanged (idempotent); a live
    /// task moves to `canceled` and its worker is signalled to abort in-flight.
    pub async fn cancel_task(&self, creation_task_id: &str) -> Result<CreationTask, AppError> {
        validate_uuidv7(creation_task_id)
            .map_err(|error| AppError::BadRequest(format!("invalid creation_task_id: {error}")))?;
        let row = self
            .repo
            .get_task(creation_task_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("creation task {creation_task_id} not found")))?;
        if TaskStatus::parse_str(&row.status).is_some_and(TaskStatus::is_terminal) {
            let mut rows = self.audit_rows_for_output(vec![row]).await?;
            return rows.pop().expect("one terminal task remains after artifact audit").try_into();
        }
        // Write the terminal status FIRST, then cancel the token so the worker's
        // finalize sees `Canceled` and won't overwrite it.
        let updated = self
            .repo
            .update_task(
                creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Canceled.as_str()),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await?;
        if let Some(token) = self.inflight.lock().unwrap().get(creation_task_id) {
            token.cancel();
        }
        updated.try_into()
    }

    fn artifact_manifest(row: &CreationTaskRow) -> (TaskArtifactManifest, Option<TaskArtifactIssue>) {
        let committed = row.status == TaskStatus::Succeeded.as_str();
        if !committed {
            return (
                TaskArtifactManifest {
                    creation_task_id: row.creation_task_id.clone(),
                    committed: false,
                    asset_ids: Vec::new(),
                },
                None,
            );
        }

        let parsed = (|| -> Result<Vec<String>, String> {
            let capability = MediaCapability::parse(&row.capability)
                .ok_or_else(|| format!("capability '{}' is invalid", row.capability))?;
            let params = serde_json::from_str::<Value>(&row.params)
                .map_err(|error| format!("params is invalid JSON: {error}"))?;
            let required_count = required_artifact_count(capability, &params)
                .map_err(|error| error.message)?;
            let ids = serde_json::from_str::<Vec<String>>(&row.result_asset_ids)
                .map_err(|error| format!("result_asset_ids is invalid JSON: {error}"))?;
            if ids.is_empty() {
                return Err("succeeded task has no result artifacts".to_string());
            }
            let mut canonical = Vec::with_capacity(ids.len());
            let mut unique = HashSet::with_capacity(ids.len());
            for id in ids {
                let id = WorkshopAssetId::parse(id)
                    .map_err(|error| format!("result asset id is invalid: {error}"))?
                    .into_string();
                if !unique.insert(id.clone()) {
                    return Err(format!("result asset id '{id}' is duplicated"));
                }
                canonical.push(id);
            }
            if canonical.len() < required_count {
                return Err(format!(
                    "succeeded task claims {} result artifact(s), but capability '{}' with its persisted params requires at least {required_count}",
                    canonical.len(),
                    capability.as_str(),
                ));
            }
            Ok(canonical)
        })();
        match parsed {
            Ok(asset_ids) => (
                TaskArtifactManifest {
                    creation_task_id: row.creation_task_id.clone(),
                    committed: true,
                    asset_ids,
                },
                None,
            ),
            Err(reason) => (
                TaskArtifactManifest {
                    creation_task_id: row.creation_task_id.clone(),
                    committed: true,
                    asset_ids: Vec::new(),
                },
                Some(TaskArtifactIssue { creation_task_id: row.creation_task_id.clone(), reason }),
            ),
        }
    }

    fn artifact_manifests(rows: &[CreationTaskRow]) -> (Vec<TaskArtifactManifest>, Vec<TaskArtifactIssue>) {
        let mut manifests = Vec::with_capacity(rows.len());
        let mut issues = Vec::new();
        for row in rows {
            let (manifest, issue) = Self::artifact_manifest(row);
            manifests.push(manifest);
            issues.extend(issue);
        }
        (manifests, issues)
    }

    fn artifact_contract_error(issues: &[TaskArtifactIssue]) -> AppError {
        let details = issues
            .iter()
            .map(|issue| format!("{}: {}", issue.creation_task_id, issue.reason))
            .collect::<Vec<_>>()
            .join("; ");
        AppError::Internal(format!(
            "managed creation artifact contract failed: {details}"
        ))
    }

    async fn verify_artifact_manifests(
        &self,
        rows: &[CreationTaskRow],
    ) -> Result<Vec<TaskArtifactManifest>, AppError> {
        let (manifests, mut issues) = Self::artifact_manifests(&rows);
        let committed = manifests
            .iter()
            .filter(|manifest| manifest.committed && !manifest.asset_ids.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if !committed.is_empty() {
            let sink = self.asset_sink.as_ref().ok_or_else(|| {
                AppError::Internal("cannot verify succeeded creation artifacts: no asset sink is configured".into())
            })?;
            issues.extend(sink.verify_task_artifacts(&committed).await.map_err(|error| {
                AppError::Internal(format!(
                    "creation artifact integrity verification failed: {}",
                    error.message
                ))
            })?);
        }
        if !issues.is_empty() {
            return Err(Self::artifact_contract_error(&issues));
        }
        Ok(manifests)
    }

    async fn audit_rows_for_output(
        &self,
        rows: Vec<CreationTaskRow>,
    ) -> Result<Vec<CreationTaskRow>, AppError> {
        self.verify_artifact_manifests(&rows).await?;
        Ok(rows)
    }

    /// Read-only startup audit for durable creation rows and their managed
    /// asset files. A failure means the current dataset is incompatible and
    /// must be retired/reset as a whole; this method never repairs rows.
    pub async fn audit_managed_data_on_boot(&self) -> Result<(), AppError> {
        let rows = self.repo.list_all_tasks().await?;
        self.verify_artifact_manifests(&rows).await?;
        Ok(())
    }

    /// Boot reconciliation ("running ⟺ active executor" invariant). Async tasks that
    /// have a remote job id are RESUMED (their poll loop restarts); every other
    /// live task (queued, or running with no remote handle) is converged to
    /// `failed(interrupted)`. Returns the count settled as failed.
    pub async fn reconcile_on_boot(self: &Arc<Self>) -> Result<usize, AppError> {
        let all_rows = self.repo.list_all_tasks().await?;
        let manifests = self.verify_artifact_manifests(&all_rows).await?;
        if let Some(sink) = self.asset_sink.as_ref() {
            let report = sink
                .reconcile_task_artifacts(&manifests)
                .await
                .map_err(|error| {
                    AppError::Internal(format!(
                        "managed creation artifact reconciliation failed: {}",
                        error.message
                    ))
                })?;
            if !report.invalid_committed_tasks.is_empty() {
                return Err(Self::artifact_contract_error(
                    &report.invalid_committed_tasks,
                ));
            }
            if report.removed_assets > 0 {
                tracing::info!(
                    removed = report.removed_assets,
                    "creation boot reconcile: removed uncommitted or orphan task assets"
                );
            }
            if !report.cleanup_failures.is_empty() {
                let details = report
                    .cleanup_failures
                    .iter()
                    .map(|failure| {
                        format!(
                            "{}:{}: {}",
                            failure
                                .creation_task_id
                                .as_deref()
                                .unwrap_or("unknown-task"),
                            failure.asset_id,
                            failure.reason
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(AppError::Internal(format!(
                    "managed creation artifact cleanup failed: {details}"
                )));
            }
        }

        let live = all_rows
            .into_iter()
            .filter(|row| matches!(row.status.as_str(), "queued" | "running"))
            .collect::<Vec<_>>();
        let mut settled = 0;
        let mut resumed = 0;
        for row in live {
            let remote = row
                .remote_task_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if row.status == TaskStatus::Running.as_str() && remote.is_some() {
                let prepared = (|| -> Result<(MediaCapability, Value, usize), CreationError> {
                    let capability = MediaCapability::parse(&row.capability).ok_or_else(|| {
                        CreationError::new(
                            "unsupported_capability",
                            format!("persisted capability '{}' is invalid", row.capability),
                        )
                    })?;
                    let params = serde_json::from_str::<Value>(&row.params).map_err(|error| {
                        CreationError::new(
                            "invalid_params",
                            format!("persisted task params is invalid JSON: {error}"),
                        )
                    })?;
                    let required_count = required_artifact_count(capability, &params)?;
                    Ok((capability, params, required_count))
                })();
                match prepared {
                    Ok((capability, params, required_artifact_count)) => {
                        self.spawn(WorkerJob {
                            creation_task_id: row.creation_task_id.clone(),
                            project_id: row.project_id,
                            workbench_kind: row
                                .workbench_kind
                                .as_deref()
                                .and_then(StandaloneWorkbenchKind::parse),
                            workflow_id: row.workflow_id,
                            workflow_run_id: row.workflow_run_id,
                            workflow_step_id: row.workflow_step_id,
                            node_id: row.node_id,
                            provider_id: row.provider_id,
                            model: row.model,
                            capability,
                            params,
                            required_artifact_count,
                            inputs: Vec::new(), // inputs already consumed at submit; poll needs none
                            submitted_at: row.submitted_at,
                            remote_task_id: remote,
                        });
                        resumed += 1;
                    }
                    Err(error) => match self.write_failed(&row.creation_task_id, &error).await {
                        Ok(()) => settled += 1,
                        Err(write_error) => tracing::warn!(
                            id = %row.creation_task_id,
                            error = %write_error,
                            "creation boot reconcile: reject invalid resumable task failed"
                        ),
                    },
                }
                continue;
            }

            let err = CreationError::new(
                "interrupted",
                "task did not survive a restart (no active executor); settled at boot",
            );
            match self.write_failed(&row.creation_task_id, &err).await {
                Ok(()) => settled += 1,
                Err(e) => tracing::warn!(id = %row.creation_task_id, error = %e, "creation boot reconcile: settle failed"),
            }
        }
        if settled > 0 || resumed > 0 {
            tracing::info!(settled, resumed, "creation boot reconcile complete");
        }
        Ok(settled)
    }

    // -----------------------------------------------------------------------
    // Worker lifecycle
    // -----------------------------------------------------------------------

    /// Register the task's cancellation token and spawn its worker (fresh or
    /// resume, distinguished by `job.remote_task_id`).
    fn spawn(self: &Arc<Self>, job: WorkerJob) {
        let token = CancellationToken::new();
        self.inflight.lock().unwrap().insert(job.creation_task_id.clone(), token.clone());
        let this = Arc::clone(self);
        let creation_task_id = job.creation_task_id.clone();
        tokio::spawn(async move {
            this.run_worker(job, token).await;
            this.inflight.lock().unwrap().remove(&creation_task_id);
        });
    }

    async fn run_worker(&self, job: WorkerJob, token: CancellationToken) {
        // Wait for a global + per-provider permit (cancellable while queued).
        let _permits = match self.acquire_permits(&job.provider_id, &token).await {
            Some(p) => p,
            None => return, // cancelled while queued (status already `canceled`)
        };
        if token.is_cancelled() {
            return;
        }
        // A fresh task transitions queued→running; a resume is already running.
        // The transition is conditional on the task still being live, so a
        // cancel that lands after acquire_permits cannot be resurrected to
        // `running` (and then finalized as succeeded).
        if job.remote_task_id.is_none() {
            match self.mark_running(&job.creation_task_id).await {
                Ok(true) => {}
                Ok(false) => return, // canceled (or gone) before we claimed running
                Err(e) => {
                    tracing::warn!(id = %job.creation_task_id, error = %e, "creation: mark running failed; abandoning task");
                    return;
                }
            }
        }

        let outcome = self.execute(&job, &token).await;
        self.finalize(&job.creation_task_id, &token, outcome).await;
    }

    async fn execute(&self, job: &WorkerJob, token: &CancellationToken) -> ExecOutcome {
        if job.capability == MediaCapability::Text {
            if job.remote_task_id.is_some() {
                return ExecOutcome::Failed(CreationError::config(
                    "text creation cannot resume a media protocol job",
                ));
            }
            let Some(executor) = self.text_executor.as_ref() else {
                return ExecOutcome::Failed(CreationError::config(
                    "no Agent Chat executor is wired into the creation engine",
                ));
            };
            let max_tokens = match param_text_max_tokens(&job.params) {
                Ok(max_tokens) => max_tokens,
                Err(error) => return ExecOutcome::Failed(error),
            };
            let request = CreationTextRequest {
                provider_id: job.provider_id.clone(),
                model: job.model.clone(),
                system: param_str(&job.params, "system").unwrap_or_default(),
                prompt: param_prompt(&job.params),
                max_tokens,
            };
            let completion = tokio::select! {
                _ = token.cancelled() => return ExecOutcome::Canceled,
                result = executor.complete(request) => result,
            };
            return match completion {
                Ok(text) => {
                    self.persist_assets_or_fail(
                        job,
                        vec![ProducedAsset {
                            data: ProducedData::Bytes(text.into_bytes()),
                            mime: Some(TEXT_MIME.to_string()),
                        }],
                    )
                    .await
                }
                Err(error) => ExecOutcome::Failed(error),
            };
        }

        let Some(invoke) = self.invoke.clone() else {
            return ExecOutcome::Failed(CreationError::config(
                "no invoke service wired into the creation engine",
            ));
        };
        // Fresh tasks load their input bytes; a resume polls with no inputs.
        let inputs = if job.remote_task_id.is_none() {
            match self.load_inputs(&job.inputs).await {
                Ok(i) => i,
                Err(e) => return ExecOutcome::Failed(e),
            }
        } else {
            Vec::new()
        };
        let req = match cap_to_task_request(job.capability, &job.params, inputs) {
            Ok(r) => r,
            Err(e) => return ExecOutcome::Failed(e),
        };
        let mref = ModelRef { provider_id: job.provider_id.clone(), model: job.model.clone() };

        if let Some(raw) = job.remote_task_id.as_deref() {
            let handle = match parse_job_handle(raw) {
                Ok(handle) => handle,
                Err(error) => return ExecOutcome::Failed(error),
            };
            return self.poll_loop(job, &invoke, &mref, &req, handle, token).await;
        }

        let outcome = tokio::select! {
            _ = token.cancelled() => return ExecOutcome::Canceled,
            r = invoke.invoke(&mref, req) => r,
        };
        match outcome {
            Err(e) => ExecOutcome::Failed(e.into()),
            Ok(TaskOutcome::Done(result)) => self.persist_or_fail(job, result).await,
            Ok(TaskOutcome::Pending(handle)) => {
                let serialized = match serde_json::to_string(&handle) {
                    Ok(s) => s,
                    Err(e) => {
                        return ExecOutcome::Failed(CreationError::config(format!(
                            "serialize job handle failed: {e}"
                        )));
                    }
                };
                match self.set_remote(&job.creation_task_id, &serialized).await {
                    Ok(true) => {}
                    Ok(false) => return ExecOutcome::Canceled,
                    Err(e) => {
                        return ExecOutcome::Failed(CreationError::config(format!(
                            "persist remote task id failed: {e}"
                        )));
                    }
                }
                // Poll rides an input-less request (mirroring a boot resume:
                // inputs were consumed at submit; polling only needs the task
                // shape), so submit-time input bytes are not held for hours.
                let poll_req = match cap_to_task_request(job.capability, &job.params, Vec::new()) {
                    Ok(r) => r,
                    Err(e) => return ExecOutcome::Failed(e),
                };
                self.poll_loop(job, &invoke, &mref, &poll_req, handle, token).await
            }
        }
    }

    async fn poll_loop(
        &self,
        job: &WorkerJob,
        invoke: &ModelInvokeService,
        mref: &ModelRef,
        req: &TaskRequest,
        mut handle: JobHandle,
        token: &CancellationToken,
    ) -> ExecOutcome {
        // A boot-resumed job (its `remote_task_id` was set at spawn from the
        // persisted row) budgets from resume time, NOT the original submit: the
        // app may have been down far longer than `task_timeout`, and an absolute
        // `submitted_at + timeout` deadline would already be elapsed, failing the
        // still-healthy remote job on the first iteration without a single poll.
        let deadline = if job.remote_task_id.is_some() {
            now_ms() + self.task_timeout.as_millis() as i64
        } else {
            job.submitted_at + self.task_timeout.as_millis() as i64
        };
        loop {
            if token.is_cancelled() {
                return ExecOutcome::Canceled;
            }
            if now_ms() >= deadline {
                return ExecOutcome::Failed(CreationError::timeout(
                    "async task exceeded its poll deadline",
                ));
            }
            tokio::select! {
                _ = token.cancelled() => return ExecOutcome::Canceled,
                _ = tokio::time::sleep(self.poll_interval) => {}
            }
            let poll = tokio::select! {
                _ = token.cancelled() => return ExecOutcome::Canceled,
                r = invoke.poll(mref, req.clone(), &handle) => r,
            };
            match poll {
                Ok(TaskOutcome::Pending(next)) => {
                    handle = next;
                    continue;
                }
                Ok(TaskOutcome::Done(result)) => return self.persist_or_fail(job, result).await,
                Err(e) => {
                    // Terminal: an upstream 4xx (bad job id / auth), a
                    // JobFailed (the remote job reached a terminal failure
                    // state — the old PollResult::Failed leg), or a catalog
                    // kind (provider/model deleted or retagged mid-poll must
                    // not spin until the deadline). 5xx / network / parse is
                    // transient — keep polling until the deadline.
                    if e.http_status.is_some_and(|s| (400..500).contains(&s))
                        || matches!(
                            e.kind,
                            InvokeErrorKind::JobFailed
                                | InvokeErrorKind::Config
                                | InvokeErrorKind::NoAdapter
                                | InvokeErrorKind::UnsupportedTask
                                | InvokeErrorKind::MissingConnection
                        )
                    {
                        return ExecOutcome::Failed(e.into());
                    }
                    tracing::warn!(id = %job.creation_task_id, error = %e.message, "creation poll transient error; retrying");
                }
            }
        }
    }

    async fn persist_or_fail(&self, job: &WorkerJob, result: TaskResult) -> ExecOutcome {
        let assets = match result {
            TaskResult::Assets(assets) => assets,
            // Defensive: no media creation capability maps to any other
            // result shape. Text creation never enters ModelInvokeService.
            _ => {
                return ExecOutcome::Failed(CreationError::new(
                    "invalid_artifact",
                    "unexpected result type from the invoke layer for a creation task",
                ));
            }
        };
        self.persist_assets_or_fail(job, assets).await
    }

    async fn persist_assets_or_fail(
        &self,
        job: &WorkerJob,
        assets: Vec<ProducedAsset>,
    ) -> ExecOutcome {
        match self.persist_assets(job, assets).await {
            Ok(ids) => ExecOutcome::Succeeded(ids),
            Err(e) => ExecOutcome::Failed(e),
        }
    }

    async fn finalize(&self, creation_task_id: &str, token: &CancellationToken, outcome: ExecOutcome) {
        match outcome {
            ExecOutcome::Canceled => {} // status already `canceled`
            ExecOutcome::Succeeded(ids) => {
                if token.is_cancelled() {
                    self.rollback_assets(creation_task_id, &ids, "cancel won before success commit").await;
                    return; // a cancel won the race; leave the `canceled` status
                }
                if ids.is_empty() {
                    let error = CreationError::new(
                        "invalid_artifact",
                        "creation engine refused a successful terminal state without persisted artifacts",
                    );
                    if let Err(write_error) = self.write_failed(creation_task_id, &error).await {
                        tracing::warn!(creation_task_id = %creation_task_id, error = %write_error, "creation: reject empty success failed");
                    }
                    return;
                }
                match self.write_succeeded(creation_task_id, &ids).await {
                    Ok(true) => {}
                    Ok(false) => {
                        self.rollback_assets(creation_task_id, &ids, "success commit lost a terminal-state race").await;
                    }
                    Err(e) => {
                        tracing::warn!(creation_task_id = %creation_task_id, error = %e, "creation: write succeeded failed");
                        self.rollback_assets(creation_task_id, &ids, "success status write failed").await;
                        let state_error = CreationError::new(
                            "state_persist",
                            format!("persisting the succeeded task state failed: {e}"),
                        );
                        if let Err(write_error) = self.write_failed(creation_task_id, &state_error).await {
                            tracing::error!(creation_task_id = %creation_task_id, error = %write_error, "creation: fallback failed-state write also failed");
                        }
                    }
                }
            }
            ExecOutcome::Failed(err) => {
                if token.is_cancelled() {
                    return;
                }
                if let Err(e) = self.write_failed(creation_task_id, &err).await {
                    tracing::warn!(creation_task_id = %creation_task_id, error = %e, "creation: write failed failed");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // IO helpers
    // -----------------------------------------------------------------------

    async fn load_inputs(&self, inputs: &[CreationInput]) -> Result<Vec<InputAsset>, CreationError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let source = self
            .asset_source
            .as_ref()
            .ok_or_else(|| CreationError::config("no asset source wired into the creation engine"))?;
        let mut out = Vec::with_capacity(inputs.len());
        for i in inputs {
            let loaded = source.load(&i.asset_id).await?;
            if !i.kind.matches_mime(&loaded.mime) {
                return Err(CreationError::new(
                    "input_kind_mismatch",
                    format!(
                        "input asset {} was bound as {}, but its MIME is {}",
                        i.asset_id,
                        i.kind.as_str(),
                        loaded.mime
                    ),
                ));
            }
            out.push(InputAsset {
                id: Some(i.asset_id.clone()),
                role: i.role.clone(),
                bytes: loaded.bytes,
                mime: loaded.mime,
            });
        }
        Ok(out)
    }

    async fn persist_assets(
        &self,
        job: &WorkerJob,
        assets: Vec<ProducedAsset>,
    ) -> Result<Vec<String>, CreationError> {
        if assets.is_empty() {
            return Err(CreationError::provider_error("adapter produced no artifacts"));
        }
        if assets.len() < job.required_artifact_count {
            return Err(CreationError::new(
                "invalid_artifact",
                format!(
                    "adapter produced {} artifact(s), but this task requires at least {}",
                    assets.len(),
                    job.required_artifact_count
                ),
            ));
        }

        // Resolve and validate the complete batch before persisting any member.
        // Otherwise a corrupt second image could fail the task after the first
        // image was already indexed as an unreachable partial result.
        let mut resolved = Vec::with_capacity(assets.len());
        for asset in assets {
            let (bytes, mime) = match asset.data {
                ProducedData::Bytes(bytes) => {
                    let mime = validate_for_capability(&bytes, asset.mime.as_deref(), job.capability)?;
                    (bytes, mime)
                }
                ProducedData::Url(url) => self.download(&url, asset.mime.as_deref(), job.capability).await?,
            };
            resolved.push((bytes, mime));
        }

        let sink = self
            .asset_sink
            .as_ref()
            .ok_or_else(|| CreationError::config("no asset sink wired into the creation engine"))?;
        let origin = build_origin(job);
        let mut ids = Vec::with_capacity(resolved.len());
        for (bytes, mime) in resolved {
            let raw_id = match sink
                .persist(PersistAsset {
                    bytes,
                    mime,
                    in_library: true, // generated products land in the library by default
                    origin: origin.clone(),
                })
                .await
            {
                Ok(id) => id,
                Err(error) => {
                    return Err(self.rollback_partial_batch(&ids, error).await);
                }
            };
            let id = match WorkshopAssetId::parse(&raw_id) {
                Ok(id) => id,
                Err(error) => {
                    // The sink did create an asset, but violated its id
                    // contract. Include the raw id so it can still undo it.
                    ids.push(raw_id);
                    let error = CreationError::config(format!("asset sink returned invalid asset id: {error}"));
                    return Err(self.rollback_partial_batch(&ids, error).await);
                }
            };
            ids.push(id.into_string());
        }
        Ok(ids)
    }

    async fn rollback_partial_batch(&self, ids: &[String], original: CreationError) -> CreationError {
        if ids.is_empty() {
            return original;
        }
        let Some(sink) = self.asset_sink.as_ref() else {
            return CreationError::new(
                "asset_rollback",
                format!("{}; rollback unavailable because no asset sink is wired", original.message),
            );
        };
        match sink.rollback(ids).await {
            Ok(()) => original,
            Err(rollback) => CreationError::new(
                "asset_rollback",
                format!("{}; rollback failed: {}", original.message, rollback.message),
            ),
        }
    }

    async fn rollback_assets(&self, creation_task_id: &str, ids: &[String], reason: &str) {
        if ids.is_empty() {
            return;
        }
        let Some(sink) = self.asset_sink.as_ref() else {
            tracing::error!(creation_task_id = %creation_task_id, asset_ids = ?ids, reason, "creation: provisional assets cannot be rolled back; sink missing");
            return;
        };
        match sink.rollback(ids).await {
            Ok(()) => tracing::info!(creation_task_id = %creation_task_id, asset_ids = ?ids, reason, "creation: provisional asset batch rolled back"),
            Err(error) => tracing::error!(creation_task_id = %creation_task_id, asset_ids = ?ids, reason, error_kind = %error.kind, error_message = %error.message, "creation: provisional asset rollback failed"),
        }
    }

    async fn download(
        &self,
        url: &str,
        mime_hint: Option<&str>,
        capability: MediaCapability,
    ) -> Result<(Vec<u8>, String), CreationError> {
        if url.trim().is_empty() {
            return Err(CreationError::new("invalid_artifact", "provider returned an empty artifact URL"));
        }
        let response = self
            .artifact_downloader
            .get(url.trim())
            .await
            .map_err(map_artifact_download_error)?;
        if !response.status.is_success() {
            // Any non-2xx on an artifact download is a provider failure,
            // regardless of the invoke layer's finer status buckets.
            let detail = String::from_utf8_lossy(&response.body);
            let detail = detail.trim();
            let message = if detail.is_empty() {
                format!(
                    "artifact download failed with HTTP {} from {}",
                    response.status,
                    redacted_url(&response.final_url)
                )
            } else {
                format!(
                    "artifact download failed with HTTP {} from {}: {}",
                    response.status,
                    redacted_url(&response.final_url),
                    detail.chars().take(1024).collect::<String>()
                )
            };
            return Err(
                CreationError::provider_error(message).with_http_status(response.status.as_u16())
            );
        }
        let response_content_type = response
            .headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        let declared_mime = reconcile_mime(mime_hint, response_content_type)?;
        let bytes = response.body;
        let mime = validate_for_capability(&bytes, declared_mime.as_deref(), capability)?;
        Ok((bytes, mime))
    }

    fn provider_sem(&self, provider_id: &str) -> Arc<Semaphore> {
        self.provider_sems
            .lock()
            .unwrap()
            .entry(provider_id.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_provider_limit)))
            .clone()
    }

    /// Acquire a global + per-provider permit, cancellable while waiting. Returns
    /// `None` if the token fires before both are held.
    async fn acquire_permits(
        &self,
        provider_id: &str,
        token: &CancellationToken,
    ) -> Option<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
        let global = tokio::select! {
            _ = token.cancelled() => return None,
            p = self.global_sem.clone().acquire_owned() => p.ok()?,
        };
        let sem = self.provider_sem(provider_id);
        let per = tokio::select! {
            _ = token.cancelled() => return None,
            p = sem.acquire_owned() => p.ok()?,
        };
        Some((global, per))
    }

    // -----------------------------------------------------------------------
    // DB state transitions (best-effort; log on failure)
    // -----------------------------------------------------------------------

    /// Transition queued→running, conditional on the task still being live.
    /// Returns `false` when a concurrent cancel already wrote a terminal status
    /// (so the worker must not proceed and resurrect it).
    async fn mark_running(&self, creation_task_id: &str) -> Result<bool, AppError> {
        let applied = self
            .repo
            .update_task_if_live(
                creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Running.as_str()),
                    started_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await?;
        Ok(applied)
    }

    async fn set_remote(&self, creation_task_id: &str, remote_task_id: &str) -> Result<bool, AppError> {
        Ok(self.repo.set_remote_task_id_if_live(creation_task_id, remote_task_id).await?)
    }

    async fn write_succeeded(&self, creation_task_id: &str, asset_ids: &[String]) -> Result<bool, AppError> {
        let ids_json = serde_json::to_string(asset_ids).unwrap_or_else(|_| "[]".to_string());
        // Conditional: never overwrite a terminal status (e.g. a `canceled` that
        // won the race with this finalize). The token check in `finalize` is a
        // cheap early-out; THIS is the correctness gate.
        let applied = self
            .repo
            .update_task_if_live(
                creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Succeeded.as_str()),
                    result_asset_ids: Some(&ids_json),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await?;
        if !applied {
            tracing::info!(creation_task_id = %creation_task_id, "creation: succeeded write skipped; task no longer live (cancel won the race)");
        }
        Ok(applied)
    }

    async fn write_failed(&self, creation_task_id: &str, err: &CreationError) -> Result<(), AppError> {
        let error_json = serde_json::to_string(err)
            .unwrap_or_else(|_| r#"{"kind":"internal","message":"error serialization failed"}"#.to_string());
        let applied = self
            .repo
            .update_task_if_live(
                creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Failed.as_str()),
                    error: Some(Some(&error_json)),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await?;
        if !applied {
            tracing::info!(creation_task_id = %creation_task_id, "creation: failed write skipped; task no longer live");
        }
        Ok(())
    }
}

fn map_artifact_download_error(error: SafeHttpError) -> CreationError {
    match error.kind() {
        SafeHttpErrorKind::InvalidUrl
        | SafeHttpErrorKind::ForbiddenTarget
        | SafeHttpErrorKind::InvalidRedirect
        | SafeHttpErrorKind::TooManyRedirects
        | SafeHttpErrorKind::BodyTooLarge => {
            CreationError::new("invalid_artifact", error.to_string())
        }
        SafeHttpErrorKind::Timeout => CreationError::timeout(error.to_string()),
        SafeHttpErrorKind::ClientBuild => CreationError::config(error.to_string()),
        SafeHttpErrorKind::Dns
        | SafeHttpErrorKind::Network
        | SafeHttpErrorKind::BodyRead => CreationError::provider_error(error.to_string()),
    }
}


/// Build the provenance object stamped onto every produced asset's `origin`.
fn build_origin(job: &WorkerJob) -> Value {
    let mut origin = serde_json::Map::from_iter([
        (
            "prompt".into(),
            Value::String(
                job.params
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
        ),
        ("model".into(), Value::String(job.model.clone())),
        (
            "provider_id".into(),
            Value::String(job.provider_id.clone()),
        ),
        (
            "capability".into(),
            Value::String(job.capability.as_str().to_owned()),
        ),
        ("params".into(), job.params.clone()),
        (
            "creation_task_id".into(),
            Value::String(job.creation_task_id.as_str().to_owned()),
        ),
    ]);
    if let Some(project_id) = &job.project_id {
        origin.insert("project_id".into(), Value::String(project_id.clone()));
    }
    if let Some(workbench_kind) = job.workbench_kind {
        origin.insert(
            "workbench_kind".into(),
            Value::String(workbench_kind.as_str().to_owned()),
        );
    }
    if let Some(workflow_id) = &job.workflow_id {
        origin.insert("workflow_id".into(), Value::String(workflow_id.clone()));
    }
    if let Some(workflow_run_id) = &job.workflow_run_id {
        origin.insert(
            "workflow_run_id".into(),
            Value::String(workflow_run_id.clone()),
        );
    }
    if let Some(workflow_step_id) = &job.workflow_step_id {
        origin.insert(
            "workflow_step_id".into(),
            Value::String(workflow_step_id.clone()),
        );
    }
    if let Some(node_id) = &job.node_id {
        origin.insert("node_id".into(), Value::String(node_id.clone()));
    }
    Value::Object(origin)
}

impl TaskStatus {
    fn parse_str(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _ => return None,
        })
    }
}

#[cfg(test)]
async fn seed_service_test_project(pool: &nomifun_db::SqlitePool) -> String {
    let project_id = CreativeStudioProjectId::new().into_string();
    let document = serde_json::json!({
        "schema": "nomifun.creative-studio/v1",
        "projectId": project_id,
        "nodes": []
    });
    sqlx::query(
        "INSERT INTO creative_studio_projects \
            (project_id, title, revision, node_count, connection_count, document_json, created_at, updated_at) \
         VALUES (?, 'Creation Service Test', 1, 0, 0, ?, 0, 0)",
    )
    .bind(&project_id)
    .bind(document.to_string())
    .execute(pool)
    .await
    .unwrap();
    project_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::ModelTask;
    use nomifun_db::{
        CreationTaskRow, DbError, IProviderRepository, IWorkshopRepository, NewProviderModel,
        NewProviderModelCapability, SqliteCreationTaskRepository,
        SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
        SqliteProviderModelRepository, SqliteProviderRepository, SqliteWorkshopRepository,
    };
    use nomifun_model_invoke::{AdapterRegistry, InvokeError, ProtocolAdapter, ResolvedCall};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore as TestSemaphore;

    const TEST_KEY: [u8; 32] = [0x42; 32];

    /// Every task the shared "test-model" row declares — the invoke layer's
    /// task-membership gate is exercised by dedicated invoke-layer tests; here
    /// the model is fully tagged so the state-machine tests stay focused.
    const ALL_TEST_TASKS: &[&str] = &[
        "image_generation",
        "image_edit",
        "video_generation",
        "chat",
        "speech_synthesis",
    ];

    fn valid_png() -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([1, 2, 3, 255]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn valid_mp4() -> Vec<u8> {
        crate::artifact::tests::bmff(b"isom")
    }

    // ---- test doubles ----

    /// A configurable invoke-level protocol adapter: synchronous `Done`, or
    /// async `Pending` then a scripted number of `Pending` polls before a
    /// terminal outcome. Registered in a private [`AdapterRegistry`] under a
    /// real protocol id (e.g. `"openai.images"`) also persisted on the seeded
    /// model capability.
    struct MockAdapter {
        id: &'static str,
        supports: Vec<ModelTask>,
        behavior: MockBehavior,
        submit_calls: AtomicUsize,
        poll_calls: AtomicUsize,
    }
    #[derive(Clone)]
    enum MockBehavior {
        DoneSync,
        DoneEmpty,
        DoneEmptyBytes,
        DoneInvalidImage,
        DoneValidThenInvalid,
        DoneTwoValid,
        DoneManyValid(usize),
        SubmitError(String),
        /// Pending on submit; return Pending for `pending_polls` polls, then Done.
        AsyncDone { pending_polls: usize },
        /// Pending on submit; never completes (each poll returns Pending).
        AsyncNever,
    }
    impl MockAdapter {
        fn sync(id: &'static str) -> Arc<Self> {
            Arc::new(Self {
                id,
                supports: vec![ModelTask::ImageGeneration, ModelTask::ImageEdit],
                behavior: MockBehavior::DoneSync,
                submit_calls: AtomicUsize::new(0),
                poll_calls: AtomicUsize::new(0),
            })
        }
        fn with(id: &'static str, supports: Vec<ModelTask>, behavior: MockBehavior) -> Arc<Self> {
            Arc::new(Self {
                id,
                supports,
                behavior,
                submit_calls: AtomicUsize::new(0),
                poll_calls: AtomicUsize::new(0),
            })
        }
        fn png_asset() -> ProducedAsset {
            ProducedAsset { data: ProducedData::Bytes(valid_png()), mime: Some("image/png".into()) }
        }
        fn pending_handle(&self) -> JobHandle {
            JobHandle {
                adapter_id: self.id.into(),
                remote_id: "remote-123".into(),
                config_revision: 1,
                poll_state: json!({}),
            }
        }
    }
    #[async_trait]
    impl ProtocolAdapter for MockAdapter {
        fn id(&self) -> &'static str {
            self.id
        }
        fn supports(&self, task: ModelTask) -> bool {
            self.supports.contains(&task)
        }
        async fn submit(&self, _http: &reqwest::Client, _call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
            self.submit_calls.fetch_add(1, Ordering::SeqCst);
            match &self.behavior {
                MockBehavior::DoneSync => {
                    Ok(TaskOutcome::Done(TaskResult::Assets(vec![Self::png_asset()])))
                }
                MockBehavior::DoneEmpty => Ok(TaskOutcome::Done(TaskResult::Assets(Vec::new()))),
                MockBehavior::DoneEmptyBytes => Ok(TaskOutcome::Done(TaskResult::Assets(vec![
                    ProducedAsset { data: ProducedData::Bytes(Vec::new()), mime: Some("image/png".into()) },
                ]))),
                MockBehavior::DoneInvalidImage => Ok(TaskOutcome::Done(TaskResult::Assets(vec![
                    ProducedAsset {
                        data: ProducedData::Bytes(b"not-an-image".to_vec()),
                        mime: Some("image/png".into()),
                    },
                ]))),
                MockBehavior::DoneValidThenInvalid => Ok(TaskOutcome::Done(TaskResult::Assets(vec![
                    Self::png_asset(),
                    ProducedAsset {
                        data: ProducedData::Bytes(b"not-an-image".to_vec()),
                        mime: Some("image/png".into()),
                    },
                ]))),
                MockBehavior::DoneTwoValid => Ok(TaskOutcome::Done(TaskResult::Assets(vec![
                    Self::png_asset(),
                    Self::png_asset(),
                ]))),
                MockBehavior::DoneManyValid(count) => Ok(TaskOutcome::Done(TaskResult::Assets(
                    (0..*count).map(|_| Self::png_asset()).collect(),
                ))),
                MockBehavior::SubmitError(m) => {
                    Err(InvokeError::new(nomifun_model_invoke::InvokeErrorKind::ProviderError, m.clone()))
                }
                MockBehavior::AsyncDone { .. } | MockBehavior::AsyncNever => {
                    Ok(TaskOutcome::Pending(self.pending_handle()))
                }
            }
        }
        async fn poll(
            &self,
            _http: &reqwest::Client,
            _call: &ResolvedCall,
            job: &JobHandle,
        ) -> Result<TaskOutcome, InvokeError> {
            let n = self.poll_calls.fetch_add(1, Ordering::SeqCst);
            match &self.behavior {
                MockBehavior::AsyncDone { pending_polls } => {
                    if n < *pending_polls {
                        Ok(TaskOutcome::Pending(job.clone()))
                    } else {
                        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
                            data: ProducedData::Bytes(valid_mp4()),
                            mime: Some("video/mp4".into()),
                        }])))
                    }
                }
                _ => Ok(TaskOutcome::Pending(job.clone())),
            }
        }
    }

    struct RecordingSink {
        count: AtomicUsize,
        origins: Mutex<Vec<Value>>,
    }

    struct RecordingTextExecutor {
        requests: Mutex<Vec<CreationTextRequest>>,
        response: String,
    }

    impl RecordingTextExecutor {
        fn new(response: &str) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                response: response.to_owned(),
            })
        }
    }

    #[async_trait]
    impl CreationTextExecutor for RecordingTextExecutor {
        async fn complete(
            &self,
            request: CreationTextRequest,
        ) -> Result<String, CreationError> {
            self.requests.lock().unwrap().push(request);
            Ok(self.response.clone())
        }
    }

    /// Transaction-aware sink used to make partial writes and cancellation
    /// windows deterministic in regression tests.
    struct TransactionalTestSink {
        persist_calls: AtomicUsize,
        rollback_calls: AtomicUsize,
        live_ids: Mutex<Vec<(String, Option<String>)>>,
        fail_on_call: Option<usize>,
        block_on_call: Option<usize>,
        entered: TestSemaphore,
        release: TestSemaphore,
        rolled_back: TestSemaphore,
    }

    impl TransactionalTestSink {
        fn new(fail_on_call: Option<usize>, block_on_call: Option<usize>) -> Arc<Self> {
            Arc::new(Self {
                persist_calls: AtomicUsize::new(0),
                rollback_calls: AtomicUsize::new(0),
                live_ids: Mutex::new(Vec::new()),
                fail_on_call,
                block_on_call,
                entered: TestSemaphore::new(0),
                release: TestSemaphore::new(0),
                rolled_back: TestSemaphore::new(0),
            })
        }

        fn live_count(&self) -> usize {
            self.live_ids.lock().unwrap().len()
        }

        fn contains(&self, asset_id: &str) -> bool {
            self.live_ids.lock().unwrap().iter().any(|(id, _)| id == asset_id)
        }
    }

    #[async_trait]
    impl AssetSink for TransactionalTestSink {
        async fn persist(&self, asset: PersistAsset) -> Result<String, CreationError> {
            let call = self.persist_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_on_call == Some(call) {
                return Err(CreationError::new("asset_write", format!("scripted persist failure #{call}")));
            }
            let id = WorkshopAssetId::new().into_string();
            let creation_task_id = asset
                .origin
                .get("creation_task_id")
                .and_then(Value::as_str)
                .and_then(|value| validate_uuidv7(value).ok().map(|_| value.to_string()));
            self.live_ids.lock().unwrap().push((id.clone(), creation_task_id));
            if self.block_on_call == Some(call) {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            Ok(id)
        }

        async fn rollback(&self, asset_ids: &[String]) -> Result<(), CreationError> {
            self.rollback_calls.fetch_add(1, Ordering::SeqCst);
            self.live_ids.lock().unwrap().retain(|(id, _)| !asset_ids.contains(id));
            self.rolled_back.add_permits(1);
            Ok(())
        }

        async fn verify_task_artifacts(
            &self,
            committed_tasks: &[TaskArtifactManifest],
        ) -> Result<Vec<TaskArtifactIssue>, CreationError> {
            let live = self.live_ids.lock().unwrap();
            let mut issues = Vec::new();
            for task in committed_tasks {
                if !task.committed {
                    continue;
                }
                if task.asset_ids.is_empty()
                    || task.asset_ids.iter().any(|asset_id| {
                        !live
                            .iter()
                            .any(|(id, origin)| id == asset_id && *origin == Some(task.creation_task_id.clone()))
                    })
                {
                    issues.push(TaskArtifactIssue {
                        creation_task_id: task.creation_task_id.clone(),
                        reason: "one or more committed assets are missing or belong to another task".into(),
                    });
                }
            }
            Ok(issues)
        }

        async fn reconcile_task_artifacts(
            &self,
            all_tasks: &[TaskArtifactManifest],
        ) -> Result<TaskArtifactReconcileReport, CreationError> {
            self.rollback_calls.fetch_add(1, Ordering::SeqCst);
            let issues = self.verify_task_artifacts(all_tasks).await?;
            let invalid = issues
                .iter()
                .map(|issue| issue.creation_task_id.clone())
                .collect::<HashSet<_>>();
            let committed = all_tasks
                .iter()
                .filter(|task| task.committed && !invalid.contains(&task.creation_task_id))
                .flat_map(|task| task.asset_ids.iter().cloned())
                .collect::<HashSet<_>>();
            let mut live = self.live_ids.lock().unwrap();
            let before = live.len();
            live.retain(|(id, origin)| origin.is_none() || committed.contains(id));
            Ok(TaskArtifactReconcileReport {
                removed_assets: before - live.len(),
                invalid_committed_tasks: issues,
                cleanup_failures: Vec::new(),
            })
        }
    }

    struct FlakyReconcileSink {
        inner: Arc<TransactionalTestSink>,
        failures_remaining: AtomicUsize,
    }

    #[async_trait]
    impl AssetSink for FlakyReconcileSink {
        async fn persist(&self, asset: PersistAsset) -> Result<String, CreationError> {
            self.inner.persist(asset).await
        }

        async fn rollback(&self, asset_ids: &[String]) -> Result<(), CreationError> {
            self.inner.rollback(asset_ids).await
        }

        async fn verify_task_artifacts(
            &self,
            committed_tasks: &[TaskArtifactManifest],
        ) -> Result<Vec<TaskArtifactIssue>, CreationError> {
            self.inner.verify_task_artifacts(committed_tasks).await
        }

        async fn reconcile_task_artifacts(
            &self,
            all_tasks: &[TaskArtifactManifest],
        ) -> Result<TaskArtifactReconcileReport, CreationError> {
            if self
                .failures_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| remaining.checked_sub(1))
                .is_ok()
            {
                return Err(CreationError::new("asset_audit", "scripted inventory scan failure"));
            }
            self.inner.reconcile_task_artifacts(all_tasks).await
        }
    }

    #[derive(Clone, Copy)]
    enum SuccessCommitFault {
        CancelWins,
        Error,
    }

    /// Repository decorator that injects the two finalize races which matter:
    /// cancel wins the terminal compare-and-set, or the status write errors.
    struct ScriptedSucceededRepo {
        inner: Arc<dyn ICreationTaskRepository>,
        fault: SuccessCommitFault,
    }

    #[async_trait]
    impl ICreationTaskRepository for ScriptedSucceededRepo {
        async fn get_or_create_creative_task(
            &self,
            params: CreateCreativeTaskParams<'_>,
        ) -> Result<nomifun_db::IdempotentCreationTask, DbError> {
            self.inner.get_or_create_creative_task(params).await
        }

        async fn get_task(&self, id: &str) -> Result<Option<CreationTaskRow>, DbError> {
            self.inner.get_task(id).await
        }

        async fn list_all_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError> {
            self.inner.list_all_tasks().await
        }

        async fn update_task(
            &self,
            id: &str,
            params: UpdateCreationTaskParams<'_>,
        ) -> Result<CreationTaskRow, DbError> {
            self.inner.update_task(id, params).await
        }

        async fn update_task_if_live(
            &self,
            id: &str,
            params: UpdateCreationTaskParams<'_>,
        ) -> Result<bool, DbError> {
            if params.status == Some(TaskStatus::Succeeded.as_str()) {
                return match self.fault {
                    SuccessCommitFault::CancelWins => {
                        self.inner
                            .update_task(
                                id,
                                UpdateCreationTaskParams {
                                    status: Some(TaskStatus::Canceled.as_str()),
                                    finished_at: Some(Some(now_ms())),
                                    ..Default::default()
                                },
                            )
                            .await?;
                        Ok(false)
                    }
                    SuccessCommitFault::Error => Err(DbError::Init("scripted success commit failure".into())),
                };
            }
            self.inner.update_task_if_live(id, params).await
        }

        async fn set_remote_task_id_if_live(&self, id: &str, remote_task_id: &str) -> Result<bool, DbError> {
            self.inner.set_remote_task_id_if_live(id, remote_task_id).await
        }

        async fn list_live_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError> {
            self.inner.list_live_tasks().await
        }
    }

    struct RemotePatchGateRepo {
        inner: Arc<dyn ICreationTaskRepository>,
        entered: TestSemaphore,
        release: TestSemaphore,
    }

    #[async_trait]
    impl ICreationTaskRepository for RemotePatchGateRepo {
        async fn get_or_create_creative_task(
            &self,
            params: CreateCreativeTaskParams<'_>,
        ) -> Result<nomifun_db::IdempotentCreationTask, DbError> {
            self.inner.get_or_create_creative_task(params).await
        }

        async fn get_task(&self, id: &str) -> Result<Option<CreationTaskRow>, DbError> {
            self.inner.get_task(id).await
        }

        async fn list_all_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError> {
            self.inner.list_all_tasks().await
        }

        async fn update_task(
            &self,
            id: &str,
            params: UpdateCreationTaskParams<'_>,
        ) -> Result<CreationTaskRow, DbError> {
            self.inner.update_task(id, params).await
        }

        async fn update_task_if_live(
            &self,
            id: &str,
            params: UpdateCreationTaskParams<'_>,
        ) -> Result<bool, DbError> {
            self.inner.update_task_if_live(id, params).await
        }

        async fn set_remote_task_id_if_live(&self, id: &str, remote_task_id: &str) -> Result<bool, DbError> {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.inner.set_remote_task_id_if_live(id, remote_task_id).await
        }

        async fn list_live_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError> {
            self.inner.list_live_tasks().await
        }
    }
    #[async_trait]
    impl AssetSink for RecordingSink {
        async fn persist(&self, asset: PersistAsset) -> Result<String, CreationError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.origins.lock().unwrap().push(asset.origin);
            Ok(WorkshopAssetId::new().into_string())
        }

        async fn rollback(&self, asset_ids: &[String]) -> Result<(), CreationError> {
            self.count.fetch_sub(asset_ids.len(), Ordering::SeqCst);
            Ok(())
        }


        async fn verify_task_artifacts(
            &self,
            _committed_tasks: &[TaskArtifactManifest],
        ) -> Result<Vec<TaskArtifactIssue>, CreationError> {
            Ok(Vec::new())
        }

        async fn reconcile_task_artifacts(
            &self,
            _all_tasks: &[TaskArtifactManifest],
        ) -> Result<TaskArtifactReconcileReport, CreationError> {
            Ok(TaskArtifactReconcileReport::default())
        }
    }

    struct StaticSource;
    #[async_trait]
    impl AssetSource for StaticSource {
        async fn load(&self, _asset_id: &str) -> Result<LoadedAsset, CreationError> {
            Ok(LoadedAsset { bytes: b"input".to_vec(), mime: "image/png".into() })
        }
    }

    // ---- harness ----

    async fn seed_provider(
        pool: &nomifun_db::SqlitePool,
        platform: &str,
        protocol: &str,
    ) -> String {
        let repo = SqliteProviderRepository::new(pool.clone());
        let encrypted = nomifun_common::encrypt_string(
            r#"{"api_keys":["sk-test-key"]}"#,
            &TEST_KEY,
        )
        .unwrap();
        let capabilities = ALL_TEST_TASKS
            .iter()
            .map(|task| NewProviderModelCapability {
                task,
                traits: "[]",
                protocol,
                connection_role: "default",
                provider_params: "{}",
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let initial_model = NewProviderModel {
            model: "test-model",
            enabled: true,
            sort_order: 0,
            description: None,
            capabilities: &capabilities,
        };
        let (row, _) = repo
            .create(
                nomifun_db::CreateProviderParams {
                provider_id: None,
                platform,
                name: "Test",
                base_url: "https://api.test.com/v1",
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
                },
                &initial_model,
                &[],
            )
            .await
            .unwrap();
        row.provider_id
    }

    /// A [`ModelInvokeService`] over the shared pool with ONLY the given
    /// protocol adapters registered. The persisted task capability selects one
    /// by its exact protocol id.
    fn invoke_over(pool: &nomifun_db::SqlitePool, adapters: Vec<Arc<dyn ProtocolAdapter>>) -> Arc<ModelInvokeService> {
        Arc::new(ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(adapters),
        ))
    }

    struct Harness {
        svc: Arc<CreationService>,
        provider_id: String,
        sink: Arc<RecordingSink>,
        text_executor: Arc<RecordingTextExecutor>,
        _db: nomifun_db::Database,
    }

    async fn harness(adapter: Arc<MockAdapter>, platform: &str) -> Harness {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_id = seed_provider(&pool, platform, adapter.id).await;
        let test_project_id = seed_service_test_project(&pool).await;
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let sink = Arc::new(RecordingSink {
            count: AtomicUsize::new(0),
            origins: Mutex::new(Vec::new()),
        });
        let text_executor = RecordingTextExecutor::new("generated text");
        let svc = CreationService::builder(repo)
            .with_test_project_id(test_project_id)
            .with_invoke(invoke_over(&pool, vec![adapter as Arc<dyn ProtocolAdapter>]))
            .with_text_executor(text_executor.clone())
            .with_asset_source(Arc::new(StaticSource))
            .with_asset_sink(sink.clone())
            .with_poll_interval(Duration::from_millis(10))
            .with_task_timeout(Duration::from_secs(30))
            .build();
        Harness { svc, provider_id, sink, text_executor, _db: db }
    }

    async fn harness_with_sink_and_repo(
        adapter: Arc<MockAdapter>,
        platform: &str,
        sink: Arc<dyn AssetSink>,
        success_commit_fault: Option<SuccessCommitFault>,
    ) -> (Arc<CreationService>, String, nomifun_db::Database) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_id = seed_provider(&pool, platform, adapter.id).await;
        let test_project_id = seed_service_test_project(&pool).await;
        let sqlite_repo: Arc<dyn ICreationTaskRepository> =
            Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let repo: Arc<dyn ICreationTaskRepository> = match success_commit_fault {
            Some(fault) => Arc::new(ScriptedSucceededRepo { inner: sqlite_repo, fault }),
            None => sqlite_repo,
        };
        let svc = CreationService::builder(repo)
            .with_test_project_id(test_project_id)
            .with_invoke(invoke_over(&pool, vec![adapter as Arc<dyn ProtocolAdapter>]))
            .with_asset_source(Arc::new(StaticSource))
            .with_asset_sink(sink)
            .with_poll_interval(Duration::from_millis(10))
            .with_task_timeout(Duration::from_secs(30))
            .build();
        (svc, provider_id, db)
    }

    async fn wait_terminal(svc: &Arc<CreationService>, creation_task_id: &str) -> CreationTask {
        for _ in 0..400 {
            let t = svc.get_task(creation_task_id).await.unwrap();
            if TaskStatus::parse_str(&t.status).is_some_and(TaskStatus::is_terminal) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("task {creation_task_id} did not reach a terminal state");
    }

    async fn create_test_task(
        svc: &CreationService,
        provider_id: &str,
        capability: &str,
        params: &str,
    ) -> String {
        let creation_task_id = generate_id();
        let project_id = svc
            .test_project_id
            .as_deref()
            .expect("test service must have a canonical project");
        let node_id = CreativeStudioNodeId::new().into_string();
        let fingerprint = serde_json::json!({"test_task_id": creation_task_id}).to_string();
        svc.repo
            .get_or_create_creative_task(CreateCreativeTaskParams {
                creation_task_id: &creation_task_id,
                owner: CreativeTaskOwnerRef::CanvasNode {
                    project_id,
                    node_id: &node_id,
                },
                provider_id,
                model: "test-model",
                capability,
                params,
                input_bindings: "[]",
                request_fingerprint: &fingerprint,
                status: TaskStatus::Queued.as_str(),
                submitted_at: now_ms(),
            })
        .await
        .unwrap();
        creation_task_id
    }

    async fn seed_test_task(
        svc: &CreationService,
        provider_id: &str,
        capability: &str,
        params: &str,
        status: &str,
        result_asset_ids: &str,
    ) -> String {
        let creation_task_id = create_test_task(svc, provider_id, capability, params).await;
        svc.repo
            .update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(status),
                    result_asset_ids: Some(result_asset_ids),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        creation_task_id
    }

    fn new_task(provider_id: &str, capability: &str) -> NewCreationTask {
        NewCreationTask {
            provider_id: provider_id.into(),
            model: "test-model".into(),
            capability: capability.into(),
            params: json!({"prompt": "a cat", "count": 1}),
            inputs: vec![],
        }
    }

    async fn seed_creative_project(pool: &nomifun_db::SqlitePool) -> String {
        let project_id = CreativeStudioProjectId::new().into_string();
        let document = json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": project_id,
            "nodes": []
        });
        SqliteWorkshopRepository::new(pool.clone())
            .create_creative_project(
                &project_id,
                "Creation Service Test",
                &document.to_string(),
                0,
            )
            .await
            .unwrap();
        project_id
    }

    async fn seed_creative_workflow_run(
        pool: &nomifun_db::SqlitePool,
    ) -> (String, String, String) {
        let workflow_id = CreativeStudioWorkflowId::new().into_string();
        let workflow_run_id = CreativeStudioWorkflowRunId::new().into_string();
        let workflow_step_id = CreativeStudioWorkflowStepId::new().into_string();
        sqlx::query(
            "INSERT INTO creative_studio_workflows \
                (workflow_id, revision, name, description, category, visibility, definition_json, \
                 created_at, updated_at) \
             VALUES (?, 1, 'Creation Workflow Test', '', '', 'private', ?, 0, 0)",
        )
        .bind(&workflow_id)
        .bind(json!({"id": workflow_id, "revision": 1}).to_string())
        .execute(pool)
        .await
        .unwrap();
        let aggregate = json!({
            "kind": "nomifun.creative-studio.workflow-run",
            "version": 1,
            "revision": 1,
            "workflowSnapshot": {"id": workflow_id, "revision": 1},
            "request": {
                "id": workflow_run_id,
                "workflowId": workflow_id,
                "workflowRevision": 1
            },
            "record": {
                "requestId": workflow_run_id,
                "workflowId": workflow_id,
                "status": "running"
            }
        });
        sqlx::query(
            "INSERT INTO creative_studio_workflow_runs \
                (workflow_run_id, workflow_id, workflow_revision, revision, status, step_ids_json, \
                 aggregate_json, created_at, updated_at) \
             VALUES (?, ?, 1, 1, 'running', ?, ?, 0, 0)",
        )
        .bind(&workflow_run_id)
        .bind(&workflow_id)
        .bind(serde_json::to_string(&[&workflow_step_id]).unwrap())
        .bind(aggregate.to_string())
        .execute(pool)
        .await
        .unwrap();
        (workflow_id, workflow_run_id, workflow_step_id)
    }

    fn creative_task(provider_id: &str, prompt: &str) -> NewCreationTask {
        NewCreationTask {
            provider_id: provider_id.to_owned(),
            model: "test-model".into(),
            capability: "t2i".into(),
            params: json!({"prompt": prompt, "count": 1}),
            inputs: vec![],
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn creative_project_response_loss_retry_has_one_worker_authority() {
        let adapter = MockAdapter::sync("openai.images");
        let h = harness(adapter.clone(), "openai").await;
        let project_id = seed_creative_project(h._db.pool()).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let idempotency_key = CreationTaskId::new().into_string();

        let first_service = h.svc.clone();
        let retry_service = h.svc.clone();
        let owner = CreativeTaskOwner::CanvasNode {
            project_id: project_id.clone(),
            node_id: node_id.clone(),
        };
        let first = first_service.create_creative_task(
            owner.clone(),
            idempotency_key.clone(),
            creative_task(&h.provider_id, "Aurora"),
        );
        let retry = retry_service.create_creative_task(
            owner.clone(),
            idempotency_key.clone(),
            creative_task(&h.provider_id, "Aurora"),
        );
        let (first, retry) = tokio::join!(first, retry);
        let first = first.unwrap();
        let retry = retry.unwrap();
        assert_eq!(first.creation_task_id, idempotency_key);
        assert_eq!(retry.creation_task_id, idempotency_key);
        assert_eq!(first.project_id.as_deref(), Some(project_id.as_str()));

        let done = wait_terminal(&h.svc, &idempotency_key).await;
        assert_eq!(done.status, "succeeded");
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 1);
        assert_eq!(h.sink.count.load(Ordering::SeqCst), 1);

        let response_loss_retry = h
            .svc
            .create_creative_task(
                owner,
                idempotency_key.clone(),
                creative_task(&h.provider_id, "Aurora"),
            )
            .await
            .unwrap();
        assert_eq!(response_loss_retry.status, "succeeded");
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 1);

        let conflict = h
            .svc
            .create_creative_task(
                CreativeTaskOwner::CanvasNode {
                    project_id,
                    node_id,
                },
                idempotency_key,
                creative_task(&h.provider_id, "Different request"),
            )
            .await
            .unwrap_err();
        assert!(matches!(&conflict, AppError::Conflict(_)));
        assert_eq!(conflict.status_code(), axum::http::StatusCode::CONFLICT);
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn standalone_task_page_is_owner_scoped_cursor_strict_and_audited() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let provider_id = seed_provider(db.pool(), "openai", "openai.videos").await;
        let project_id = seed_service_test_project(db.pool()).await;
        let repo = SqliteCreationTaskRepository::new(db.pool().clone());
        let mut task_ids = Vec::new();
        for submitted_at in [200, 100] {
            let task_id = CreationTaskId::new().into_string();
            let fingerprint = json!({"task": task_id}).to_string();
            repo.get_or_create_creative_task(CreateCreativeTaskParams {
                creation_task_id: &task_id,
                owner: CreativeTaskOwnerRef::StandaloneWorkbench {
                    project_id: &project_id,
                    workbench_kind: "video",
                },
                provider_id: &provider_id,
                model: "test-model",
                capability: "t2v",
                params: r#"{"prompt":"Aurora","seconds":5}"#,
                input_bindings: "[]",
                request_fingerprint: &fingerprint,
                status: "queued",
                submitted_at,
            })
            .await
            .unwrap();
            repo.update_task(
                &task_id,
                UpdateCreationTaskParams {
                    status: Some("failed"),
                    error: Some(Some(r#"{"kind":"provider_error","message":"fixture"}"#)),
                    finished_at: Some(Some(submitted_at + 1)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            task_ids.push(task_id);
        }
        let service = CreationService::new(Arc::new(repo.clone()));

        let first = service
            .list_standalone_workbench_tasks(
                &project_id,
                StandaloneWorkbenchKind::Video,
                false,
                Some(1),
                None,
            )
            .await
            .unwrap();
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].creation_task_id, task_ids[0]);
        let cursor = first.next_cursor.expect("one more row requires a cursor");

        let second = service
            .list_standalone_workbench_tasks(
                &project_id,
                StandaloneWorkbenchKind::Video,
                false,
                Some(1),
                Some(&cursor),
            )
            .await
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].creation_task_id, task_ids[1]);
        assert!(second.next_cursor.is_none());
        assert!(
            service
                .list_standalone_workbench_tasks(
                    &project_id,
                    StandaloneWorkbenchKind::Image,
                    false,
                    None,
                    None,
                )
                .await
                .unwrap()
                .items
                .is_empty()
        );
        for invalid in ["", "01:bad", "-1:0190f5fe-7c00-7a00-8000-000000000001"] {
            assert!(matches!(
                service
                    .list_standalone_workbench_tasks(
                        &project_id,
                        StandaloneWorkbenchKind::Video,
                        false,
                        None,
                        Some(invalid),
                    )
                    .await,
                Err(AppError::BadRequest(_))
            ));
        }

        let retired = service
            .retire_standalone_workbench_tasks(
                &project_id,
                StandaloneWorkbenchKind::Video,
                &[task_ids[0].clone()],
            )
            .await
            .unwrap();
        assert_eq!(retired, vec![task_ids[0].clone()]);
        let direct = service.get_task(&task_ids[0]).await.unwrap();
        let first_deleted_at = direct.deleted_at.expect("direct GET exposes tombstone");
        assert!(
            service
                .list_standalone_workbench_tasks(
                    &project_id,
                    StandaloneWorkbenchKind::Video,
                    false,
                    None,
                    None,
                )
                .await
                .unwrap()
                .items
                .iter()
                .all(|task| task.creation_task_id != task_ids[0])
        );
        service
            .retire_standalone_workbench_tasks(
                &project_id,
                StandaloneWorkbenchKind::Video,
                &[task_ids[0].clone()],
            )
            .await
            .unwrap();
        assert_eq!(
            service.get_task(&task_ids[0]).await.unwrap().deleted_at,
            Some(first_deleted_at),
            "idempotent retire preserves the first tombstone timestamp"
        );

        let corrupt_id = CreationTaskId::new().into_string();
        let fingerprint = json!({"task": corrupt_id}).to_string();
        repo.get_or_create_creative_task(CreateCreativeTaskParams {
            creation_task_id: &corrupt_id,
            owner: CreativeTaskOwnerRef::StandaloneWorkbench {
                project_id: &project_id,
                workbench_kind: "video",
            },
            provider_id: &provider_id,
            model: "test-model",
            capability: "t2v",
            params: r#"{"prompt":"missing output","seconds":5}"#,
            input_bindings: "[]",
            request_fingerprint: &fingerprint,
            status: "queued",
            submitted_at: 300,
        })
        .await
        .unwrap();
        let missing_asset = WorkshopAssetId::new().into_string();
        let result_ids = serde_json::to_string(&[missing_asset]).unwrap();
        repo.update_task(
            &corrupt_id,
            UpdateCreationTaskParams {
                status: Some("succeeded"),
                result_asset_ids: Some(&result_ids),
                finished_at: Some(Some(301)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            service
                .retire_standalone_workbench_tasks(
                    &project_id,
                    StandaloneWorkbenchKind::Video,
                    &[corrupt_id.clone()],
                )
                .await
                .is_err(),
            "retirement must not hide a succeeded task before artifact audit"
        );
        assert!(repo.get_task(&corrupt_id).await.unwrap().unwrap().deleted_at.is_none());
    }

    #[tokio::test]
    async fn standalone_active_page_excludes_every_terminal_status() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let provider_id = seed_provider(db.pool(), "openai", "openai.images").await;
        let project_id = seed_service_test_project(db.pool()).await;
        let repo = SqliteCreationTaskRepository::new(db.pool().clone());
        let queued_id = CreationTaskId::new().into_string();
        let failed_id = CreationTaskId::new().into_string();
        for (task_id, submitted_at) in [(&queued_id, 20), (&failed_id, 10)] {
            let fingerprint = json!({"task": task_id}).to_string();
            repo.get_or_create_creative_task(CreateCreativeTaskParams {
                creation_task_id: task_id,
                owner: CreativeTaskOwnerRef::StandaloneWorkbench {
                    project_id: &project_id,
                    workbench_kind: "image",
                },
                provider_id: &provider_id,
                model: "image-model",
                capability: "t2i",
                params: r#"{"prompt":"Aurora"}"#,
                input_bindings: "[]",
                request_fingerprint: &fingerprint,
                status: "queued",
                submitted_at,
            })
            .await
            .unwrap();
        }
        repo.update_task(
            &failed_id,
            UpdateCreationTaskParams {
                status: Some("failed"),
                error: Some(Some(r#"{"kind":"provider_error","message":"fixture"}"#)),
                finished_at: Some(Some(11)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let service = CreationService::new(Arc::new(repo));

        let active = service
            .list_standalone_workbench_tasks(
                &project_id,
                StandaloneWorkbenchKind::Image,
                true,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(active.items.len(), 1);
        assert_eq!(active.items[0].creation_task_id, queued_id);

        let all = service
            .list_standalone_workbench_tasks(
                &project_id,
                StandaloneWorkbenchKind::Image,
                false,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(all.items.len(), 2);
    }

    #[tokio::test]
    async fn creative_workflow_task_runs_with_exact_owner_and_provenance() {
        let adapter = MockAdapter::sync("openai.images");
        let h = harness(adapter.clone(), "openai").await;
        let (workflow_id, workflow_run_id, workflow_step_id) =
            seed_creative_workflow_run(h._db.pool()).await;
        let creation_task_id = CreationTaskId::new().into_string();

        let created = h
            .svc
            .create_creative_task(
                CreativeTaskOwner::WorkflowStep {
                    workflow_id: workflow_id.clone(),
                    workflow_run_id: workflow_run_id.clone(),
                    workflow_step_id: workflow_step_id.clone(),
                },
                creation_task_id.clone(),
                creative_task(&h.provider_id, "Workflow Aurora"),
            )
            .await
            .unwrap();
        assert_eq!(created.workflow_id.as_deref(), Some(workflow_id.as_str()));
        assert_eq!(
            created.workflow_run_id.as_deref(),
            Some(workflow_run_id.as_str())
        );
        assert_eq!(
            created.workflow_step_id.as_deref(),
            Some(workflow_step_id.as_str())
        );
        assert!(created.project_id.is_none());
        assert!(created.node_id.is_none());

        let done = wait_terminal(&h.svc, &creation_task_id).await;
        assert_eq!(done.status, "succeeded");
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 1);
        let origins = h.sink.origins.lock().unwrap();
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0]["workflow_id"], workflow_id);
        assert_eq!(origins[0]["workflow_run_id"], workflow_run_id);
        assert_eq!(origins[0]["workflow_step_id"], workflow_step_id);
        assert_eq!(origins[0]["creation_task_id"], creation_task_id);
        assert!(origins[0].get("project_id").is_none());
        assert!(origins[0].get("canvas_id").is_none());
        assert!(origins[0].get("node_id").is_none());
    }

    #[tokio::test]
    async fn sync_task_succeeds_and_persists_asset() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2i")).await.unwrap();
        assert_eq!(created.status, "queued");
        validate_uuidv7(&created.creation_task_id).unwrap();

        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded");
        assert_eq!(done.result_asset_ids.len(), 1);
        WorkshopAssetId::parse(&done.result_asset_ids[0]).unwrap();
        assert!(done.finished_at.is_some());
        assert_eq!(h.sink.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn text_task_uses_agent_executor_and_never_media_adapter() {
        let adapter = MockAdapter::with(
            "must-not-run",
            vec![ModelTask::Chat],
            MockBehavior::SubmitError("media adapter must not execute Chat".into()),
        );
        let h = harness(adapter.clone(), "openai").await;
        let mut task = new_task(&h.provider_id, "text");
        task.params = json!({
            "prompt": "draft a launch note",
            "system": "be concise",
            "max_tokens": 777
        });

        let created = h.svc.create_test_task(task).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;

        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(h.sink.count.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 0);
        let requests = h.text_executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].provider_id, h.provider_id);
        assert_eq!(requests[0].model, "test-model");
        assert_eq!(requests[0].system, "be concise");
        assert_eq!(requests[0].prompt, "draft a launch note");
        assert_eq!(requests[0].max_tokens, 777);
    }

    #[tokio::test]
    async fn successful_provider_response_without_artifacts_fails_task() {
        let adapter = MockAdapter::with(
            "openai.images",
            vec![ModelTask::ImageGeneration],
            MockBehavior::DoneEmpty,
        );
        let h = harness(adapter, "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2i")).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert!(done.result_asset_ids.is_empty());
        assert_eq!(done.error.as_ref().unwrap()["kind"], "provider_error");
        assert_eq!(h.sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn empty_or_invalid_image_bytes_never_reach_asset_sink() {
        for behavior in [MockBehavior::DoneEmptyBytes, MockBehavior::DoneInvalidImage] {
            let adapter = MockAdapter::with("openai.images", vec![ModelTask::ImageGeneration], behavior);
            let h = harness(adapter, "openai").await;
            let created = h.svc.create_test_task(new_task(&h.provider_id, "t2i")).await.unwrap();
            let done = wait_terminal(&h.svc, &created.creation_task_id).await;
            assert_eq!(done.status, "failed");
            assert!(done.result_asset_ids.is_empty());
            assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
            assert_eq!(h.sink.count.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn invalid_batch_member_is_rejected_before_any_asset_is_persisted() {
        let adapter = MockAdapter::with(
            "openai.images",
            vec![ModelTask::ImageGeneration],
            MockBehavior::DoneValidThenInvalid,
        );
        let h = harness(adapter, "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2i")).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
        assert_eq!(h.sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn second_persist_failure_rolls_back_first_provisional_asset() {
        let adapter = MockAdapter::with(
            "openai.images",
            vec![ModelTask::ImageGeneration],
            MockBehavior::DoneTwoValid,
        );
        let sink = TransactionalTestSink::new(Some(2), None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink.clone(), None).await;

        let created = svc.create_test_task(new_task(&provider_id, "t2i")).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;

        assert_eq!(done.status, "failed");
        assert!(done.result_asset_ids.is_empty());
        assert_eq!(done.error.as_ref().unwrap()["kind"], "asset_write");
        assert_eq!(sink.persist_calls.load(Ordering::SeqCst), 2);
        assert_eq!(sink.rollback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(sink.live_count(), 0, "the first provisional asset must be removed");
    }

    #[tokio::test]
    async fn cancel_during_persist_rolls_back_completed_provisional_write() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, Some(1));
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink.clone(), None).await;

        let created = svc.create_test_task(new_task(&provider_id, "t2i")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), sink.entered.acquire())
            .await
            .expect("persist did not enter its cancellation window")
            .unwrap()
            .forget();
        assert_eq!(sink.live_count(), 1, "test must observe the provisional asset before cancel");

        let canceled = svc.cancel_task(&created.creation_task_id).await.unwrap();
        assert_eq!(canceled.status, "canceled");
        sink.release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(2), sink.rolled_back.acquire())
            .await
            .expect("worker did not roll the canceled batch back")
            .unwrap()
            .forget();

        assert_eq!(svc.get_task(&created.creation_task_id).await.unwrap().status, "canceled");
        assert_eq!(sink.live_count(), 0);
        assert_eq!(sink.rollback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn succeeded_status_write_failure_rolls_back_provisional_assets() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink.clone(), Some(SuccessCommitFault::Error)).await;

        let created = svc.create_test_task(new_task(&provider_id, "t2i")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), sink.rolled_back.acquire())
            .await
            .expect("status-write failure did not roll the batch back")
            .unwrap()
            .forget();

        let task = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(task.status, "failed");
        assert_eq!(task.error.as_ref().unwrap()["kind"], "state_persist");
        assert!(task.result_asset_ids.is_empty());
        assert_eq!(sink.live_count(), 0);
        assert_eq!(sink.rollback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancel_winning_terminal_compare_and_set_rolls_back_assets() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) = harness_with_sink_and_repo(
            adapter,
            "openai",
            sink.clone(),
            Some(SuccessCommitFault::CancelWins),
        )
        .await;

        let created = svc.create_test_task(new_task(&provider_id, "t2i")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), sink.rolled_back.acquire())
            .await
            .expect("lost terminal compare-and-set did not roll the batch back")
            .unwrap()
            .forget();

        let task = svc.get_task(&created.creation_task_id).await.unwrap();
        assert_eq!(task.status, "canceled");
        assert!(task.result_asset_ids.is_empty());
        assert_eq!(sink.live_count(), 0);
        assert_eq!(sink.rollback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn async_task_polls_then_succeeds() {
        let adapter = MockAdapter::with(
            "openai.videos",
            vec![ModelTask::VideoGeneration],
            MockBehavior::AsyncDone { pending_polls: 2 },
        );
        let h = harness(adapter, "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2v")).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded");
        assert_eq!(done.result_asset_ids.len(), 1);
        // remote task id was persisted on the way through
        let row = h.svc.get_task(&created.creation_task_id).await.unwrap();
        assert_eq!(row.status, "succeeded");
    }

    #[tokio::test]
    async fn submit_error_fails_task() {
        let adapter = MockAdapter::with(
            "openai.images",
            vec![ModelTask::ImageGeneration],
            MockBehavior::SubmitError("boom".into()),
        );
        let h = harness(adapter, "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2i")).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "provider_error");
        assert!(done.error.as_ref().unwrap()["message"].as_str().unwrap().contains("boom"));
    }

    #[tokio::test]
    async fn cancel_interrupts_running_async_task() {
        let adapter = MockAdapter::with(
            "openai.videos",
            vec![ModelTask::VideoGeneration],
            MockBehavior::AsyncNever,
        );
        let h = harness(adapter, "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2v")).await.unwrap();

        // Wait until it is running (submitted → pending → polling).
        let mut running = false;
        for _ in 0..200 {
            if h.svc.get_task(&created.creation_task_id).await.unwrap().status == "running" {
                running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(running, "task never reached running");

        let canceled = h.svc.cancel_task(&created.creation_task_id).await.unwrap();
        assert_eq!(canceled.status, "canceled");
        // Stays canceled (worker must not overwrite with succeeded/failed).
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(h.svc.get_task(&created.creation_task_id).await.unwrap().status, "canceled");
    }

    #[tokio::test]
    async fn cancel_racing_remote_id_patch_cannot_resurrect_running_status() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_id = seed_provider(&pool, "openai", "openai.videos").await;
        let test_project_id = seed_service_test_project(&pool).await;
        let inner: Arc<dyn ICreationTaskRepository> =
            Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let gated = Arc::new(RemotePatchGateRepo {
            inner,
            entered: TestSemaphore::new(0),
            release: TestSemaphore::new(0),
        });
        let adapter = MockAdapter::with(
            "openai.videos",
            vec![ModelTask::VideoGeneration],
            MockBehavior::AsyncNever,
        );
        let sink = Arc::new(RecordingSink {
            count: AtomicUsize::new(0),
            origins: Mutex::new(Vec::new()),
        });
        let svc = CreationService::builder(gated.clone())
            .with_test_project_id(test_project_id)
            .with_invoke(invoke_over(&pool, vec![adapter as Arc<dyn ProtocolAdapter>]))
            .with_asset_source(Arc::new(StaticSource))
            .with_asset_sink(sink)
            .with_poll_interval(Duration::from_millis(10))
            .build();
        let created = svc.create_test_task(new_task(&provider_id, "t2v")).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), gated.entered.acquire())
            .await
            .expect("worker never reached remote-id CAS")
            .unwrap()
            .forget();
        let canceled = svc.cancel_task(&created.creation_task_id).await.unwrap();
        assert_eq!(canceled.status, "canceled");
        gated.release.add_permits(1);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let row = svc.repo.get_task(&created.creation_task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "canceled");
        assert_eq!(row.remote_task_id, None, "CAS after cancel must not patch the terminal row");
    }

    #[tokio::test]
    async fn cancel_is_idempotent_on_terminal() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;
        let created = h.svc.create_test_task(new_task(&h.provider_id, "t2i")).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded");
        // cancel of a terminal task returns it unchanged
        let after = h.svc.cancel_task(&created.creation_task_id).await.unwrap();
        assert_eq!(after.status, "succeeded");
        let missing = generate_id();
        assert!(matches!(h.svc.cancel_task(&missing).await.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn malformed_entity_ids_are_rejected() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;
        let mut bad = new_task(&h.provider_id, "nope");
        assert!(matches!(h.svc.create_test_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        bad = new_task("  ", "t2i");
        assert!(matches!(h.svc.create_test_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        bad = new_task(&h.provider_id, "t2i");
        bad.inputs = vec![CreationInput {
            asset_id: String::new(),
            kind: CreationInputKind::Image,
            role: "reference".into(),
        }];
        assert!(matches!(h.svc.create_test_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        for owner in [
            CreativeTaskOwner::CanvasNode {
                project_id: "not-a-project".into(),
                node_id: CreativeStudioNodeId::new().into_string(),
            },
            CreativeTaskOwner::CanvasNode {
                project_id: h.svc.test_project_id.clone().unwrap(),
                node_id: "not-a-node".into(),
            },
        ] {
            assert!(matches!(
                h.svc
                    .create_creative_task(
                        owner,
                        CreationTaskId::new().into_string(),
                        new_task(&h.provider_id, "t2i"),
                    )
                    .await
                    .unwrap_err(),
                AppError::BadRequest(_)
            ));
        }
        for invalid_creation_task_id in [
            "0",
            "1",
            "task_0190f5fe-7c00-7a00-8000-000000000001",
            "0190f5fe-7c00-4a00-8000-000000000001",
            "0190F5FE-7C00-7A00-8000-000000000001",
            "0190f5fe7c007a008000000000000001",
            "0190f5fe-7c00-7a00-8000-000000000001 ",
        ] {
            assert!(matches!(
                h.svc.get_task(invalid_creation_task_id).await.unwrap_err(),
                AppError::BadRequest(_)
            ));
            assert!(matches!(
                h.svc
                    .cancel_task(invalid_creation_task_id)
                    .await
                    .unwrap_err(),
                AppError::BadRequest(_)
            ));
        }
    }

    #[tokio::test]
    async fn create_task_enforces_image_count_and_n_without_defaulting_or_clamping() {
        let adapter = MockAdapter::with(
            "openai.images",
            vec![ModelTask::ImageGeneration],
            MockBehavior::DoneManyValid(10),
        );
        let h = harness(adapter.clone(), "openai").await;
        for params in [
            json!({"prompt": "cat", "count": 0}),
            json!({"prompt": "cat", "count": -1}),
            json!({"prompt": "cat", "count": 1.5}),
            json!({"prompt": "cat", "count": "2"}),
            json!({"prompt": "cat", "count": 11}),
            json!({"prompt": "cat", "n": 0}),
            json!({"prompt": "cat", "n": 11}),
            json!({"prompt": "cat", "count": 2, "n": 3}),
        ] {
            let mut task = new_task(&h.provider_id, "t2i");
            task.params = params;
            assert!(
                matches!(h.svc.create_test_task(task).await.unwrap_err(), AppError::BadRequest(_)),
                "invalid image quantity must be rejected before enqueue"
            );
        }
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 0);
        assert!(h.svc.repo.list_all_tasks().await.unwrap().is_empty());

        // The supported ceiling is accepted verbatim (including the `n`
        // alias), and the worker enforces that same prevalidated value.
        let mut task = new_task(&h.provider_id, "t2i");
        task.params = json!({"prompt": "cat", "count": 10, "n": 10});
        let created = h.svc.create_test_task(task).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(done.result_asset_ids.len(), 10);
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn missing_provider_conflicts_before_task_persistence() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;
        let missing_provider = ProviderId::new().into_string();
        assert!(matches!(
            h.svc.create_test_task(new_task(&missing_provider, "t2i")).await.unwrap_err(),
            AppError::Conflict(_)
        ));
        assert!(h.svc.repo.list_all_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn boot_reconcile_single_inventory_scan_removes_live_and_missing_task_assets() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink.clone(), None).await;
        let queued_id = create_test_task(&svc, &provider_id, "t2i", "{}").await;
        let running_id = create_test_task(&svc, &provider_id, "t2i", "{}").await;
        svc.repo
            .update_task(
                &running_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Running.as_str()),
                    started_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let queued_asset = sink
            .persist(PersistAsset {
                bytes: valid_png(),
                mime: "image/png".into(),
                in_library: true,
                origin: json!({"creation_task_id": queued_id}),
            })
            .await
            .unwrap();
        let running_asset = sink
            .persist(PersistAsset {
                bytes: valid_png(),
                mime: "image/png".into(),
                in_library: true,
                origin: json!({"creation_task_id": running_id}),
            })
            .await
            .unwrap();
        let unrelated_task = generate_id();
        let unrelated_asset = sink
            .persist(PersistAsset {
                bytes: valid_png(),
                mime: "image/png".into(),
                in_library: true,
                origin: json!({"creation_task_id": unrelated_task}),
            })
            .await
            .unwrap();

        assert_eq!(svc.reconcile_on_boot().await.unwrap(), 2);
        assert_eq!(svc.get_task(&queued_id).await.unwrap().status, "failed");
        assert_eq!(svc.get_task(&running_id).await.unwrap().status, "failed");
        assert!(!sink.contains(&queued_asset));
        assert!(!sink.contains(&running_asset));
        assert!(!sink.contains(&unrelated_asset), "assets for a missing task must also be removed");

        // Re-running complete-inventory recovery is idempotent.
        assert_eq!(svc.reconcile_on_boot().await.unwrap(), 0);
        assert_eq!(sink.live_count(), 0);
    }

    #[tokio::test]
    async fn boot_asset_cleanup_failure_aborts_before_state_recovery_and_retries_next_pass() {
        let adapter = MockAdapter::sync("openai.images");
        let tracked = TransactionalTestSink::new(None, None);
        let flaky = Arc::new(FlakyReconcileSink {
            inner: tracked.clone(),
            failures_remaining: AtomicUsize::new(1),
        });
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", flaky.clone(), None).await;
        let creation_task_id = create_test_task(&svc, &provider_id, "t2i", "{}").await;
        let asset_id = flaky
            .persist(PersistAsset {
                bytes: valid_png(),
                mime: "image/png".into(),
                in_library: true,
                origin: json!({"creation_task_id": creation_task_id}),
            })
            .await
            .unwrap();

        let error = svc.reconcile_on_boot().await.unwrap_err();
        assert!(error.to_string().contains("artifact reconciliation failed"));
        assert_eq!(
            svc.repo
                .get_task(&creation_task_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "queued",
            "a failed managed-data audit must not partially mutate task state"
        );
        assert!(tracked.contains(&asset_id));

        assert_eq!(svc.reconcile_on_boot().await.unwrap(), 1);
        assert_eq!(
            svc.repo
                .get_task(&creation_task_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "failed"
        );
        assert!(!tracked.contains(&asset_id));
    }

    #[tokio::test]
    async fn boot_inventory_fails_closed_before_cleanup_for_invalid_committed_task() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink.clone(), None).await;

        let invalid_success =
            seed_test_task(&svc, &provider_id, "t2i", "{}", "succeeded", "[]").await;
        let invalid_success_asset = sink
            .persist(PersistAsset {
                bytes: valid_png(),
                mime: "image/png".into(),
                in_library: true,
                origin: json!({"creation_task_id": invalid_success}),
            })
            .await
            .unwrap();

        let missing_task = generate_id();
        let missing_task_asset = sink
            .persist(PersistAsset {
                bytes: valid_png(),
                mime: "image/png".into(),
                in_library: true,
                origin: json!({"creation_task_id": missing_task}),
            })
            .await
            .unwrap();

        let error = svc.reconcile_on_boot().await.unwrap_err();
        assert!(error.to_string().contains("succeeded task has no result artifacts"));
        assert!(sink.contains(&invalid_success_asset));
        assert!(sink.contains(&missing_task_asset));
        let unchanged = svc.repo.get_task(&invalid_success).await.unwrap().unwrap();
        assert_eq!(unchanged.status, "succeeded");
        assert_eq!(unchanged.result_asset_ids, "[]");
        assert!(unchanged.error.is_none());
    }

    #[tokio::test]
    async fn query_fails_closed_when_succeeded_task_claims_missing_asset() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink, None).await;
        let creation_task_id = create_test_task(&svc, &provider_id, "t2i", "{}").await;
        let missing_asset = WorkshopAssetId::new().into_string();
        let ids_json = serde_json::to_string(&[missing_asset]).unwrap();
        svc.repo
            .update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Succeeded.as_str()),
                    result_asset_ids: Some(&ids_json),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let error = svc.get_task(&creation_task_id).await.unwrap_err();
        assert!(error.to_string().contains("committed assets are missing"));
        let unchanged = svc.repo.get_task(&creation_task_id).await.unwrap().unwrap();
        assert_eq!(unchanged.status, "succeeded");
        assert_eq!(unchanged.result_asset_ids, ids_json);
    }

    #[tokio::test]
    async fn task_reads_fail_closed_for_short_image_successes_without_rewriting_rows() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;

        async fn seed_short_success(
            svc: &CreationService,
            provider_id: &str,
            params: &str,
            result_count: usize,
        ) -> String {
            let id = create_test_task(svc, provider_id, "t2i", params).await;
            let asset_ids = (0..result_count)
                .map(|_| WorkshopAssetId::new().into_string())
                .collect::<Vec<_>>();
            let asset_ids = serde_json::to_string(&asset_ids).unwrap();
            svc.repo
                .update_task(
                    &id,
                    UpdateCreationTaskParams {
                        status: Some(TaskStatus::Succeeded.as_str()),
                        result_asset_ids: Some(&asset_ids),
                        finished_at: Some(Some(now_ms())),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            id
        }

        let get_id = seed_short_success(&h.svc, &h.provider_id, r#"{"count":2}"#, 1).await;
        let get_error = h.svc.get_task(&get_id).await.unwrap_err();
        assert!(get_error.to_string().contains("requires at least 2"));
        assert_eq!(
            h.svc.repo.get_task(&get_id).await.unwrap().unwrap().status,
            "succeeded"
        );

        let second_id = seed_short_success(&h.svc, &h.provider_id, r#"{"n":3}"#, 2).await;
        let second_error = h.svc.get_task(&second_id).await.unwrap_err();
        assert!(second_error.to_string().contains("requires at least 3"));
    }

    #[tokio::test]
    async fn boot_reconciliation_rejects_short_or_invalid_count_successes_without_cleanup() {
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) = harness_with_sink_and_repo(
            MockAdapter::sync("openai.images"),
            "openai",
            sink.clone(),
            None,
        )
        .await;

        async fn seed_with_one_asset(
            svc: &CreationService,
            sink: &TransactionalTestSink,
            provider_id: &str,
            params: &str,
        ) -> (String, String) {
            let id = create_test_task(svc, provider_id, "t2i", params).await;
            let asset_id = sink
                .persist(PersistAsset {
                    bytes: valid_png(),
                    mime: "image/png".into(),
                    in_library: true,
                    origin: json!({"creation_task_id": id}),
                })
                .await
                .unwrap();
            let asset_ids = serde_json::to_string(&[&asset_id]).unwrap();
            svc.repo
                .update_task(
                    &id,
                    UpdateCreationTaskParams {
                        status: Some(TaskStatus::Succeeded.as_str()),
                        result_asset_ids: Some(&asset_ids),
                        finished_at: Some(Some(now_ms())),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            (id, asset_id)
        }

        let (short_id, short_asset) =
            seed_with_one_asset(&svc, sink.as_ref(), &provider_id, r#"{"count":2}"#).await;
        let (invalid_id, invalid_asset) =
            seed_with_one_asset(&svc, sink.as_ref(), &provider_id, r#"{"count":0}"#).await;

        let error = svc.reconcile_on_boot().await.unwrap_err();
        assert!(error.to_string().contains("managed creation artifact contract failed"));
        for (creation_task_id, asset_id) in [
            (short_id, short_asset.as_str()),
            (invalid_id, invalid_asset.as_str()),
        ] {
            let row = svc.repo.get_task(&creation_task_id).await.unwrap().unwrap();
            assert_eq!(row.status, "succeeded");
            assert!(row.error.is_none());
            assert!(sink.contains(asset_id));
        }
    }

    #[tokio::test]
    async fn cancel_endpoint_fails_closed_for_invalid_terminal_success() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink, None).await;
        let creation_task_id = create_test_task(&svc, &provider_id, "t2i", "{}").await;
        let missing_asset = WorkshopAssetId::new().into_string();
        let ids_json = serde_json::to_string(&[missing_asset]).unwrap();
        svc.repo
            .update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Succeeded.as_str()),
                    result_asset_ids: Some(&ids_json),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let error = svc.cancel_task(&creation_task_id).await.unwrap_err();
        assert!(error.to_string().contains("committed assets are missing"));
        let unchanged = svc.repo.get_task(&creation_task_id).await.unwrap().unwrap();
        assert_eq!(unchanged.status, "succeeded");
        assert_eq!(unchanged.result_asset_ids, ids_json);
    }

    #[tokio::test]
    async fn reconcile_settles_queued_and_resumes_running_with_remote() {
        // Build a service whose adapter completes on the first poll, so a resumed
        // running-with-remote task reaches succeeded.
        let adapter = MockAdapter::with(
            "openai.videos",
            vec![ModelTask::VideoGeneration],
            MockBehavior::AsyncDone { pending_polls: 0 },
        );
        let h = harness(adapter, "openai").await;
        let repo = &h.svc.repo;
        let queued_id = create_test_task(&h.svc, &h.provider_id, "t2i", "{}").await;
        let running_id = create_test_task(&h.svc, &h.provider_id, "t2v", "{}").await;
        let resume_id = create_test_task(&h.svc, &h.provider_id, "t2v", "{}").await;

        // (a) a queued leftover → should become failed(interrupted)

        // (b) a running task WITHOUT remote → failed(interrupted)
        repo.update_task(&running_id, UpdateCreationTaskParams { status: Some("running"), ..Default::default() })
            .await
            .unwrap();

        // (c) a running task WITH remote → resumed → succeeded
        let resume_handle = serde_json::to_string(&JobHandle {
            adapter_id: "openai.videos".into(),
            remote_id: "remote-xyz".into(),
            config_revision: 0,
            poll_state: json!({}),
        })
        .unwrap();
        repo.update_task(
            &resume_id,
            UpdateCreationTaskParams {
                status: Some("running"),
                remote_task_id: Some(Some(&resume_handle)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let settled = h.svc.reconcile_on_boot().await.unwrap();
        assert_eq!(settled, 2, "queued + running-without-remote settle as failed");

        assert_eq!(h.svc.get_task(&queued_id).await.unwrap().status, "failed");
        assert_eq!(
            h.svc.get_task(&queued_id).await.unwrap().error.unwrap()["kind"],
            "interrupted"
        );
        assert_eq!(h.svc.get_task(&running_id).await.unwrap().status, "failed");

        // resumed one completes via its poll loop
        let resumed = wait_terminal(&h.svc, &resume_id).await;
        assert_eq!(resumed.status, "succeeded");
    }

    #[tokio::test]
    async fn reconcile_resumed_task_uses_fresh_deadline_not_stale_submitted_at() {
        // A resumable async task whose remote completes on the first poll.
        let adapter = MockAdapter::with(
            "openai.videos",
            vec![ModelTask::VideoGeneration],
            MockBehavior::AsyncDone { pending_polls: 0 },
        );
        let h = harness(adapter, "openai").await; // task_timeout = 30s
        let repo = &h.svc.repo;

        // submitted far in the past: an absolute (submitted_at + timeout)
        // deadline would already be elapsed, so the old code would fail this on
        // the first loop iteration WITHOUT ever polling the healthy remote job.
        let old = now_ms() - 3_600_000; // 1h ago
        let old_resume_id = generate_id();
        let node_id = CreativeStudioNodeId::new().into_string();
        let fingerprint = serde_json::json!({"test_task_id": old_resume_id}).to_string();
        repo
            .get_or_create_creative_task(CreateCreativeTaskParams {
                creation_task_id: &old_resume_id,
                owner: CreativeTaskOwnerRef::CanvasNode {
                    project_id: h.svc.test_project_id.as_deref().unwrap(),
                    node_id: &node_id,
                },
                provider_id: &h.provider_id,
                model: "test-model",
                capability: "t2v",
                params: "{}",
                input_bindings: "[]",
                request_fingerprint: &fingerprint,
                status: "queued",
                submitted_at: old,
            })
            .await
            .unwrap();
        let resume_handle = serde_json::to_string(&JobHandle {
            adapter_id: "openai.videos".into(),
            remote_id: "remote-old".into(),
            config_revision: 0,
            poll_state: json!({}),
        })
        .unwrap();
        repo.update_task(
            &old_resume_id,
            UpdateCreationTaskParams {
                status: Some("running"),
                remote_task_id: Some(Some(&resume_handle)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let settled = h.svc.reconcile_on_boot().await.unwrap();
        assert_eq!(settled, 0, "the resumable task is resumed, not settled as failed");
        // With a resume-relative deadline it polls to completion instead of an
        // instant timeout.
        let done = wait_terminal(&h.svc, &old_resume_id).await;
        assert_eq!(done.status, "succeeded", "resumed old job polls to completion; error={:?}", done.error);
    }

    #[tokio::test]
    async fn bare_service_without_adapter_fails_config() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let provider_id = seed_provider(db.pool(), "openai", "openai.images").await;
        let test_project_id = seed_service_test_project(db.pool()).await;
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(db.pool().clone()));
        Box::leak(Box::new(db));
        let svc = CreationService::builder(repo)
            .with_test_project_id(test_project_id)
            .build();
        let created = svc.create_test_task(new_task(&provider_id, "t2i")).await.unwrap();
        assert_eq!(created.status, "queued");
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        // No provider repo wired → resolution fails with a config error.
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "config");
    }

    #[test]
    fn build_origin_carries_provenance() {
        let creation_task_id = generate_id();
        let project_id = CreativeStudioProjectId::new().into_string();
        let node_id = CreativeStudioNodeId::new().into_string();
        let provider_id = ProviderId::new().into_string();
        let job = WorkerJob {
            creation_task_id: creation_task_id.clone(),
            project_id: Some(project_id.clone()),
            workbench_kind: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            node_id: Some(node_id.clone()),
            provider_id: provider_id.clone(),
            model: "gpt-image-1".into(),
            capability: MediaCapability::T2i,
            params: json!({"prompt": "sunset", "count": 2}),
            required_artifact_count: 2,
            inputs: vec![],
            submitted_at: 1,
            remote_task_id: None,
        };
        let o = build_origin(&job);
        assert_eq!(o["prompt"], "sunset");
        assert_eq!(o["model"], "gpt-image-1");
        assert_eq!(o["provider_id"], provider_id);
        assert_eq!(o["project_id"], project_id);
        assert_eq!(o["node_id"], node_id);
        assert_eq!(o["creation_task_id"], creation_task_id.as_str());
        assert!(
            o.get("task_id").is_none(),
            "only creation_task_id is valid in Workshop Asset origin"
        );
        assert_eq!(o["capability"], "t2i");
        assert_eq!(o["params"]["count"], 2);
    }

    #[test]
    fn build_origin_omits_absent_owner_branch_ids_instead_of_writing_null() {
        let job = WorkerJob {
            creation_task_id: CreationTaskId::new().into_string(),
            project_id: None,
            workbench_kind: None,
            workflow_id: Some(CreativeStudioWorkflowId::new().into_string()),
            workflow_run_id: Some(CreativeStudioWorkflowRunId::new().into_string()),
            workflow_step_id: Some(CreativeStudioWorkflowStepId::new().into_string()),
            node_id: None,
            provider_id: ProviderId::new().into_string(),
            model: "gpt-image-1".into(),
            capability: MediaCapability::T2i,
            params: json!({"prompt": "sunset"}),
            required_artifact_count: 1,
            inputs: vec![],
            submitted_at: 1,
            remote_task_id: None,
        };

        let origin = build_origin(&job);
        assert!(!origin.as_object().unwrap().contains_key("project_id"));
        assert!(!origin.as_object().unwrap().contains_key("node_id"));
    }

    #[test]
    fn build_origin_carries_exact_standalone_workbench_branch() {
        let project_id = CreativeStudioProjectId::new().into_string();
        let job = WorkerJob {
            creation_task_id: CreationTaskId::new().into_string(),
            project_id: Some(project_id.clone()),
            workbench_kind: Some(StandaloneWorkbenchKind::Video),
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            node_id: None,
            provider_id: ProviderId::new().into_string(),
            model: "video-model".into(),
            capability: MediaCapability::I2v,
            params: json!({"prompt": "Aurora", "seconds": 5}),
            required_artifact_count: 1,
            inputs: vec![],
            submitted_at: 1,
            remote_task_id: None,
        };

        let origin = build_origin(&job);
        assert_eq!(origin["project_id"], project_id);
        assert_eq!(origin["workbench_kind"], "video");
        assert!(origin.get("node_id").is_none());
        assert!(origin.get("workflow_id").is_none());
    }

    // ---- param helpers (ported verbatim from the retired adapters/mod.rs) ----

    #[test]
    fn param_helpers() {
        let p = serde_json::json!({"prompt": "a cat", "width": 512, "height": 768, "count": 3});
        assert_eq!(param_prompt(&p), "a cat");
        assert_eq!(param_count(&p).unwrap(), 3);
        assert_eq!(param_size(&p).as_deref(), Some("512x768"));

        let p2 = serde_json::json!({"size": "1024x1024", "count": MAX_IMAGE_OUTPUT_COUNT});
        assert_eq!(param_size(&p2).as_deref(), Some("1024x1024"));
        assert_eq!(param_count(&p2).unwrap(), MAX_IMAGE_OUTPUT_COUNT);
        assert_eq!(param_count(&serde_json::json!({})).unwrap(), 1); // default
        assert_eq!(param_count(&serde_json::json!({"n": 4})).unwrap(), 4);
        assert_eq!(param_count(&serde_json::json!({"count": 4, "n": 4})).unwrap(), 4);
        for invalid in [
            serde_json::json!({"count": 0}),
            serde_json::json!({"count": -1}),
            serde_json::json!({"count": "2"}),
            serde_json::json!({"count": 1.5}),
            serde_json::json!({"n": MAX_IMAGE_OUTPUT_COUNT + 1}),
            serde_json::json!({"count": 2, "n": 3}),
        ] {
            assert!(param_count(&invalid).is_err(), "must reject {invalid}");
        }
        assert_eq!(param_prompt(&serde_json::json!({})), "");
        assert!(param_size(&serde_json::json!({})).is_none());
    }

    // ---- capability → TaskRequest mapping ----

    #[test]
    fn cap_to_task_request_maps_every_media_capability() {
        let params = json!({
            "prompt": "a cat", "count": 2, "width": 512, "height": 512,
            "quality": "high", "seconds": 4, "voice": "alloy", "system": "be brief"
        });
        let input = InputAsset { id: None, role: "mask".into(), bytes: vec![1], mime: "image/png".into() };

        match cap_to_task_request(MediaCapability::T2i, &params, vec![]).unwrap() {
            TaskRequest::ImageGeneration(r) => {
                assert_eq!(r.prompt, "a cat");
                assert_eq!(r.count, 2);
                assert_eq!(r.size.as_deref(), Some("512x512"));
                assert_eq!(r.quality.as_deref(), Some("high"));
                assert_eq!(
                    r.extra,
                    json!({"seconds": 4, "voice": "alloy", "system": "be brief"})
                );
            }
            _ => panic!("t2i must map to ImageGeneration"),
        }
        for cap in [MediaCapability::I2i, MediaCapability::Inpaint] {
            match cap_to_task_request(cap, &params, vec![input.clone()]).unwrap() {
                TaskRequest::ImageEdit(r) => {
                    assert_eq!(r.count, 2);
                    assert_eq!(r.inputs.len(), 1);
                    assert_eq!(r.inputs[0].role, "mask");
                }
                _ => panic!("{cap:?} must map to ImageEdit"),
            }
        }
        for cap in [MediaCapability::T2v, MediaCapability::I2v] {
            match cap_to_task_request(cap, &params, vec![]).unwrap() {
                TaskRequest::VideoGeneration(r) => {
                    assert_eq!(r.seconds, Some(4));
                    assert_eq!(r.size.as_deref(), Some("512x512"));
                    assert_eq!(
                        r.extra,
                        json!({"count": 2, "quality": "high", "voice": "alloy", "system": "be brief"})
                    );
                }
                _ => panic!("{cap:?} must map to VideoGeneration"),
            }
        }
        // Numeric-string seconds tolerated (legacy openai_video behavior).
        match cap_to_task_request(MediaCapability::T2v, &json!({"seconds": "8"}), vec![]).unwrap() {
            TaskRequest::VideoGeneration(r) => assert_eq!(r.seconds, Some(8)),
            _ => unreachable!(),
        }
        // Present-but-unparseable seconds is a typed local error, not a
        // silent drop (the old code forwarded garbage to the provider).
        for bad in [json!({"seconds": "soon"}), json!({"seconds": -1}), json!({"seconds": 1.5})] {
            let Err(err) = cap_to_task_request(MediaCapability::T2v, &bad, vec![]) else {
                panic!("unparseable seconds must be rejected: {bad}");
            };
            assert_eq!(err.kind, "invalid_params");
            assert!(err.message.contains("seconds"), "message: {}", err.message);
        }
        match cap_to_task_request(MediaCapability::Tts, &params, vec![]).unwrap() {
            TaskRequest::SpeechSynthesis(r) => {
                assert_eq!(r.text, "a cat");
                assert_eq!(r.voice.as_deref(), Some("alloy"));
            }
            _ => panic!("tts must map to SpeechSynthesis"),
        }
        let Err(text_error) = cap_to_task_request(MediaCapability::Text, &params, vec![]) else {
            panic!("text must never map to a media invocation request");
        };
        assert_eq!(text_error.kind, "config");
        assert!(text_error.message.contains("Agent Chat"));
        let Err(err) = cap_to_task_request(MediaCapability::V2v, &params, vec![]) else {
            panic!("v2v must be rejected");
        };
        assert_eq!(err.kind, "unsupported_capability");
        let Err(err) = cap_to_task_request(MediaCapability::T2i, &json!({"count": 0}), vec![]) else {
            panic!("count 0 must be rejected");
        };
        assert_eq!(err.kind, "invalid_params");
    }

    #[test]
    fn canonical_video_fields_and_canvas_metadata_never_reach_provider_extra() {
        let params = json!({
            "prompt": "camera move",
            "seconds": 5,
            "width": 1920,
            "height": 1080,
            "resolution": "1080p",
            "aspect": "16:9",
            "canvasOperation": "video-node-compose",
            "sourceNodeId": "video-node",
            "sourceAssetId": null
        });
        let TaskRequest::VideoGeneration(request) =
            cap_to_task_request(MediaCapability::T2v, &params, vec![]).unwrap()
        else {
            panic!("t2v must map to VideoGeneration");
        };
        assert_eq!(request.size.as_deref(), Some("1920x1080"));
        assert_eq!(request.seconds, Some(5));
        assert_eq!(
            request.extra,
            json!({})
        );
    }

    #[test]
    fn canonical_tts_fields_and_audio_canvas_metadata_never_reach_provider_extra() {
        let params = json!({
            "prompt": "literal narration",
            "text": "legacy duplicate that must not be forwarded",
            "voice": "alloy",
            "format": "mp3",
            "speed": 1.25,
            "instructions": "warm and calm",
            "canvasOperation": "audio-node-compose",
            "sourceNodeId": "audio-node",
            "sourceAssetId": null
        });
        let TaskRequest::SpeechSynthesis(request) =
            cap_to_task_request(MediaCapability::Tts, &params, vec![]).unwrap()
        else {
            panic!("tts must map to SpeechSynthesis");
        };
        assert_eq!(request.text, "literal narration");
        assert_eq!(request.voice.as_deref(), Some("alloy"));
        assert_eq!(request.format.as_deref(), Some("mp3"));
        assert_eq!(
            request.extra,
            json!({
                "speed": 1.25,
                "instructions": "warm and calm"
            })
        );
    }

    #[test]
    fn text_max_tokens_is_strict_and_bounded() {
        assert_eq!(param_text_max_tokens(&json!({})).unwrap(), DEFAULT_TEXT_MAX_TOKENS);
        assert_eq!(param_text_max_tokens(&json!({"max_tokens": 8192})).unwrap(), 8192);
        for invalid in [
            json!({"max_tokens": 0}),
            json!({"max_tokens": -1}),
            json!({"max_tokens": "4096"}),
            json!({"max_tokens": u64::from(u32::MAX) + 1}),
        ] {
            let error = param_text_max_tokens(&invalid).unwrap_err();
            assert_eq!(error.kind, "invalid_params", "input={invalid}");
        }
    }

    // ---- JobHandle persistence contract ----

    #[test]
    fn parse_job_handle_requires_complete_json() {
        let handle = JobHandle {
            adapter_id: "openai.videos".into(),
            remote_id: "vid_1".into(),
            config_revision: 1,
            poll_state: json!({"k": 1}),
        };
        let raw = serde_json::to_string(&handle).unwrap();
        let parsed = parse_job_handle(&raw).unwrap();
        assert_eq!(parsed.adapter_id, "openai.videos");
        assert_eq!(parsed.remote_id, "vid_1");
        assert_eq!(parsed.config_revision, 1);
        assert_eq!(parsed.poll_state, json!({"k": 1}));

        for invalid in [
            "vid-bare-id",
            r#"{"adapter_id":"","remote_id":"vid_1"}"#,
            r#"{"adapter_id":"openai.videos","remote_id":""}"#,
        ] {
            let error = parse_job_handle(invalid).unwrap_err();
            assert_eq!(error.kind, "config", "input={invalid}");
        }
    }

    #[test]
    fn invoke_error_maps_onto_creation_error_vocabulary() {
        use nomifun_model_invoke::InvokeErrorKind as K;
        for (kind, want) in [
            (K::UnsupportedTask, "unsupported_capability"),
            (K::InvalidParams, "invalid_params"),
            (K::Timeout, "timeout"),
            (K::Config, "config"),
            (K::MissingConnection, "config"),
            (K::NoAdapter, "adapter_unavailable"),
            (K::Auth, "provider_error"),
            (K::ProviderError, "provider_error"),
            (K::JobFailed, "provider_error"),
            (K::Network, "provider_error"),
            (K::ParseError, "provider_error"),
            (K::RateLimited, "provider_error"),
            (K::QuotaExhausted, "provider_error"),
            (K::ContentPolicy, "provider_error"),
            (K::NotPollable, "provider_error"),
        ] {
            let e: CreationError = InvokeError::new(kind, "boom").into();
            assert_eq!(e.kind, want, "kind {kind:?}");
            assert_eq!(e.message, "boom");
        }
        let mut src = InvokeError::new(K::ProviderError, "upstream said no");
        src.http_status = Some(404);
        let e: CreationError = src.into();
        assert_eq!(e.http_status, Some(404), "http_status must transfer");
    }
}

/// End-to-end tests driving the **real invoke-layer adapters** through the
/// engine against a wiremock HTTP server — verifies request construction +
/// response parsing + artifact persistence over the wire (no live network).
#[cfg(test)]
mod http_e2e_tests {
    use super::*;
    use base64::Engine as _;
    use nomifun_db::{
        IProviderModelRepository, IProviderRepository, NewProviderModel,
        NewProviderModelCapability, SqliteCreationTaskRepository,
        SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
        SqliteProviderModelRepository, SqliteProviderRepository,
    };
    use nomifun_model_invoke::AdapterRegistry;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_KEY: [u8; 32] = [0x37; 32];

    fn valid_png() -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([4, 5, 6, 255]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn valid_mp4() -> Vec<u8> {
        crate::artifact::tests::bmff(b"isom")
    }

    /// Two minimal valid MPEG1-Layer3 frames (mirrors the artifact test
    /// fixture) — the TTS e2e needs bytes `validate_audio` accepts as mp3.
    fn valid_mp3() -> Vec<u8> {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        frame[10] = 1;
        [frame.clone(), frame].concat()
    }

    struct CountingSink {
        count: AtomicUsize,
        /// Captured `(mime, bytes)` of each persisted artifact — lets the text
        /// e2e assert the produced MIME + body without the real bridge.
        persisted: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
    }
    #[async_trait]
    impl AssetSink for CountingSink {
        async fn persist(&self, asset: PersistAsset) -> Result<String, CreationError> {
            assert!(!asset.bytes.is_empty(), "persisted asset must carry bytes");
            self.persisted.lock().unwrap().push((asset.mime.clone(), asset.bytes.clone()));
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(WorkshopAssetId::new().into_string())
        }

        async fn rollback(&self, asset_ids: &[String]) -> Result<(), CreationError> {
            self.count.fetch_sub(asset_ids.len(), Ordering::SeqCst);
            let mut persisted = self.persisted.lock().unwrap();
            let keep = persisted.len().saturating_sub(asset_ids.len());
            persisted.truncate(keep);
            Ok(())
        }


        async fn verify_task_artifacts(
            &self,
            _committed_tasks: &[TaskArtifactManifest],
        ) -> Result<Vec<TaskArtifactIssue>, CreationError> {
            Ok(Vec::new())
        }

        async fn reconcile_task_artifacts(
            &self,
            _all_tasks: &[TaskArtifactManifest],
        ) -> Result<TaskArtifactReconcileReport, CreationError> {
            Ok(TaskArtifactReconcileReport::default())
        }
    }
    struct NoInputs;
    #[async_trait]
    impl AssetSource for NoInputs {
        async fn load(&self, _id: &str) -> Result<LoadedAsset, CreationError> {
            Err(CreationError::new("no_input", "no inputs in these tests"))
        }
    }

    async fn build(base_url: &str) -> (Arc<CreationService>, String, Arc<CountingSink>, nomifun_db::Database) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        // seed a provider row pointed at the mock server
        let prov_repo = SqliteProviderRepository::new(pool.clone());
        let encrypted =
            nomifun_common::encrypt_string(r#"{"api_keys":["sk-e2e"]}"#, &TEST_KEY)
                .unwrap();
        let image_capabilities = [
            NewProviderModelCapability {
                task: "image_generation",
                traits: "[]",
                protocol: "openai.images",
                connection_role: "default",
                provider_params: "{}",
                ..Default::default()
            },
            NewProviderModelCapability {
                task: "image_edit",
                traits: "[]",
                protocol: "openai.images",
                connection_role: "default",
                provider_params: "{}",
                ..Default::default()
            },
        ];
        let initial_model = NewProviderModel {
            model: "gpt-image-1",
            enabled: true,
            sort_order: 0,
            description: None,
            capabilities: &image_capabilities,
        };
        let provider = prov_repo
            .create(
                nomifun_db::CreateProviderParams {
                provider_id: None,
                platform: "openai",
                name: "Mock",
                base_url,
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
                },
                &initial_model,
                &[],
            )
            .await
            .unwrap()
            .0;
        let provider_id = provider.provider_id;
        let mut config_revision = provider.config_revision;
        // Save every remaining model with its complete task-scoped transport.
        let model_repo = SqliteProviderModelRepository::new(pool.clone());
        for (model, task, protocol) in [
            ("sora-2", "video_generation", "openai.videos"),
            ("tts-1", "speech_synthesis", "openai.audio_speech"),
        ] {
            let capabilities = [NewProviderModelCapability {
                task,
                traits: "[]",
                protocol,
                connection_role: "default",
                provider_params: "{}",
                ..Default::default()
            }];
            model_repo
                .save(
                    &provider_id,
                    config_revision,
                    &NewProviderModel {
                        model,
                        enabled: true,
                        sort_order: 0,
                        description: None,
                        capabilities: &capabilities,
                    },
                )
                .await
                .unwrap();
            config_revision += 1;
        }
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let test_project_id = seed_service_test_project(&pool).await;
        // Both provider calls and artifact downloads target loopback WireMock;
        // neither may inherit a developer-machine HTTP proxy.
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let invoke = Arc::new(ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            http.clone(),
            AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
        ));
        let sink = Arc::new(CountingSink { count: AtomicUsize::new(0), persisted: std::sync::Mutex::new(Vec::new()) });
        let svc = CreationService::builder(repo)
            .with_test_project_id(test_project_id)
            .with_invoke(invoke)
            .with_artifact_downloader_for_tests(
                SafeHttpClient::new(DOWNLOAD_TIMEOUT, MAX_ARTIFACT_BYTES as usize)
                    .allow_private_for_tests()
                    .user_agent("NomiFun-Creation-Test/1.0"),
            )
            .with_asset_source(Arc::new(NoInputs))
            .with_asset_sink(sink.clone())
            .with_poll_interval(Duration::from_millis(10))
            .with_task_timeout(Duration::from_secs(30))
            .build();
        (svc, provider_id, sink, db)
    }

    async fn wait_terminal(svc: &Arc<CreationService>, creation_task_id: &str) -> CreationTask {
        for _ in 0..400 {
            let t = svc.get_task(creation_task_id).await.unwrap();
            if TaskStatus::parse_str(&t.status).is_some_and(TaskStatus::is_terminal) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("task {creation_task_id} never terminated");
    }

    fn t2i(provider_id: &str) -> NewCreationTask {
        NewCreationTask {
            provider_id: provider_id.into(),
            model: "gpt-image-1".into(),
            capability: "t2i".into(),
            params: json!({"prompt": "a fox", "width": 512, "height": 512, "count": 1}),
            inputs: vec![],
        }
    }

    #[tokio::test]
    async fn openai_images_end_to_end() {
        let server = MockServer::start().await;
        let encoded = base64::engine::general_purpose::STANDARD.encode(valid_png());
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": encoded}]})))
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_test_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(done.result_asset_ids.len(), 1);
        WorkshopAssetId::parse(&done.result_asset_ids[0]).unwrap();
        assert_eq!(sink.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn openai_images_cannot_complete_with_fewer_products_than_requested() {
        let server = MockServer::start().await;
        let encoded = base64::engine::general_purpose::STANDARD.encode(valid_png());
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"b64_json": encoded}]})),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let mut request = t2i(&provider_id);
        request.params["count"] = json!(4);
        let created = svc.create_test_task(request).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;

        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
        assert!(done.error.as_ref().unwrap()["message"]
            .as_str()
            .is_some_and(|message| message.contains("requires at least 4")));
        assert!(done.result_asset_ids.is_empty());
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn openai_images_rejects_empty_artifact_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"url": "  "}]})))
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_test_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn openai_images_rejects_html_download_disguised_as_success() {
        let server = MockServer::start().await;
        let artifact_url = format!("{}/artifact.png", server.uri());
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"url": artifact_url}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifact.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html; charset=utf-8")
                    .set_body_string("<!doctype html><title>upstream error</title>"),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_test_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn openai_images_downloads_and_validates_real_url_artifact() {
        let server = MockServer::start().await;
        let artifact_url = format!("{}/artifact.png", server.uri());
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"url": artifact_url}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifact.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(valid_png()),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_test_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(done.result_asset_ids.len(), 1);
        assert_eq!(sink.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn openai_images_rejects_download_content_type_mismatch() {
        let server = MockServer::start().await;
        let artifact_url = format!("{}/artifact.png", server.uri());
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"url": artifact_url}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifact.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .set_body_bytes(valid_png()),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_test_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn openai_images_propagates_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let (svc, provider_id, _sink, _db) = build(&server.uri()).await;
        let created = svc.create_test_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        let err = done.error.unwrap();
        assert_eq!(err["kind"], "provider_error");
        assert_eq!(err["http_status"], 401);
    }

    #[tokio::test]
    async fn openai_video_submit_poll_content_end_to_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "queued"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos/vid_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "completed"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos/vid_1/content"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .set_body_bytes(valid_mp4()),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            provider_id: provider_id.clone(),
            model: "sora-2".into(),
            capability: "t2v".into(),
            params: json!({"prompt": "a wave", "seconds": 4}),
            inputs: vec![],
        };
        let created = svc.create_test_task(task).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(sink.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn openai_video_remote_failed_status_fails_task_without_spinning() {
        // A remote job reaching a terminal "failed" status must fail the task
        // on the FIRST poll (the old PollResult::Failed leg), preserving the
        // provider's failure reason — not spin as transient until the
        // deadline. The expect(1) counts prove no re-poll; no /content mock
        // is mounted, so a content fetch would 404 the poll chain.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "v1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "v1", "status": "failed", "error": {"message": "moderation blocked"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            provider_id: provider_id.clone(),
            model: "sora-2".into(),
            capability: "t2v".into(),
            params: json!({"prompt": "a wave", "seconds": 4}),
            inputs: vec![],
        };
        let created = svc.create_test_task(task).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        let err = done.error.as_ref().unwrap();
        assert_eq!(err["kind"], "provider_error");
        assert!(
            err["message"].as_str().is_some_and(|m| m.contains("moderation blocked")),
            "provider failure reason must survive: {err}"
        );
        assert!(done.result_asset_ids.is_empty());
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tts_end_to_end_produces_audio_artifact() {
        // Tts is now routable: capability "tts" maps to SpeechSynthesis →
        // The test provider base is an origin-only preset, so the task path is
        // appended verbatim rather than silently injecting `/v1`.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/speech"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(valid_mp3()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            provider_id: provider_id.clone(),
            model: "tts-1".into(),
            capability: "tts".into(),
            params: json!({"prompt": "hello there", "voice": "alloy"}),
            inputs: vec![],
        };
        let created = svc.create_test_task(task).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(done.result_asset_ids.len(), 1);
        let persisted = sink.persisted.lock().unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].0, "audio/mpeg", "mime={}", persisted[0].0);
        assert_eq!(persisted[0].1, valid_mp3());
    }

    #[tokio::test]
    async fn untagged_model_fails_with_unsupported_capability() {
        // Gate tightening (planned new behavior): a task against a model that
        // is not tagged for the capability's task gets a typed
        // `unsupported_capability` error instead of hitting a wrong endpoint.
        let server = MockServer::start().await;
        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let mut request = t2i(&provider_id);
        request.model = "gpt-4o-mini".into(); // tagged ["chat"], not image_generation
        let created = svc.create_test_task(request).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "unsupported_capability");
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
        assert!(server.received_requests().await.unwrap().is_empty(), "gate must fire before the wire");
    }
}
