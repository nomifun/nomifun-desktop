//! [`CreationService`] — the generation task queue + state machine (contract §6
//! `service.rs`).
//!
//! The service owns the full lifecycle: `queued → running →
//! succeeded/failed/canceled`, a per-provider concurrency gate + a global cap,
//! synchronous and async (submit→poll) protocols, cancellation propagation,
//! boot reconciliation, and handing produced bytes to an [`AssetSink`]. Model
//! execution is delegated to the unified invocation layer
//! ([`nomifun_model_invoke::ModelInvokeService`]): a capability + params map to
//! a typed [`TaskRequest`], and provider/protocol resolution happens inside the
//! invoke layer against the model catalog.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_common::{
    AppError, CreationTaskId, ProviderId, WorkshopAssetId, WorkshopCanvasId, WorkshopNodeId,
    now_ms, validate_uuidv7,
};
#[cfg(test)]
use nomifun_common::generate_id;
use nomifun_db::{
    CreateCreationTaskParams, CreationTaskRow, ICreationTaskRepository,
    ListCreationTasksParams, UpdateCreationTaskParams,
};
use nomifun_model_invoke::{
    ChatTextRequest, ImageEditRequest, ImageGenRequest, InputAsset, InvokeErrorKind, JobHandle,
    MAX_ARTIFACT_BYTES, ModelInvokeService, ModelRef, ProducedAsset, ProducedData, TaskOutcome,
    TaskRequest, TaskResult, TtsRequest, VideoGenRequest, error_from_response, net_err,
    read_body_capped,
};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::artifact::{reconcile_mime, validate_for_capability};
use crate::dto::CreationTask;
use crate::types::{CreationError, CreationInput, MediaCapability, TaskStatus};

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
            extra: params.clone(),
        }),
        MediaCapability::I2i | MediaCapability::Inpaint => TaskRequest::ImageEdit(ImageEditRequest {
            prompt: param_prompt(params),
            count: param_count(params)?,
            size: param_size(params),
            inputs,
            extra: params.clone(),
        }),
        MediaCapability::T2v | MediaCapability::I2v => TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: param_prompt(params),
            seconds: param_seconds(params)?,
            size: param_size(params),
            inputs,
            extra: params.clone(),
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
            extra: params.clone(),
        }),
        MediaCapability::Text => TaskRequest::ChatText(ChatTextRequest {
            prompt: param_prompt(params),
            system: param_str(params, "system"),
            extra: params.clone(),
        }),
    })
}

/// The invoke protocol that owned a LEGACY (pre-JobHandle) persisted remote
/// task id. The only async protocol before the invoke migration was the
/// OpenAI-compatible `/videos` submit→poll — every other capability was
/// synchronous and never persisted a remote id.
fn legacy_default_adapter_for(_capability: MediaCapability) -> &'static str {
    "openai.videos"
}

/// Parse a persisted `remote_task_id` column value into a [`JobHandle`]:
/// current rows carry the serialized handle JSON; legacy rows carried the bare
/// provider-side id, which is wrapped with the capability's legacy protocol.
fn parse_job_handle(raw: &str, capability: MediaCapability) -> JobHandle {
    serde_json::from_str::<JobHandle>(raw).unwrap_or_else(|_| JobHandle {
        adapter_id: legacy_default_adapter_for(capability).to_string(),
        remote_id: raw.to_string(),
        poll_state: serde_json::json!({}),
    })
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

/// A generation request accepted by [`CreationService::create_task`].
pub struct NewCreationTask {
    pub canvas_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    /// Wire capability code (`t2i|i2i|…`).
    pub capability: String,
    /// Opaque parameter map (prompt/size/quality/…).
    pub params: Value,
    pub inputs: Vec<CreationInput>,
}

/// A produced artifact ready for persistence: resolved bytes (URL artifacts are
/// fetched by the engine first) + MIME + provenance.
pub struct PersistAsset {
    pub bytes: Vec<u8>,
    pub mime: String,
    /// Whether the produced asset appears in the asset library. Generated
    /// products default to `true` (see [`CreationService::persist_assets`]).
    pub in_library: bool,
    /// `{prompt,model,provider_id,params,canvas_id,node_id,creation_task_id}`.
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
    canvas_id: Option<String>,
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
    /// Carries the raw persisted column value: serialized [`JobHandle`] JSON,
    /// or a legacy bare remote id (see [`parse_job_handle`]).
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
    http: reqwest::Client,
    asset_source: Option<Arc<dyn AssetSource>>,
    asset_sink: Option<Arc<dyn AssetSink>>,
    global_sem: Arc<Semaphore>,
    per_provider_limit: usize,
    provider_sems: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// Live task id → its cancellation token (present while queued/running).
    inflight: Mutex<HashMap<String, CancellationToken>>,
    poll_interval: Duration,
    task_timeout: Duration,
}

/// Builder for [`CreationService`] (the app wires the invoke layer + sink).
pub struct CreationServiceBuilder {
    repo: Arc<dyn ICreationTaskRepository>,
    invoke: Option<Arc<ModelInvokeService>>,
    http: Option<reqwest::Client>,
    asset_source: Option<Arc<dyn AssetSource>>,
    asset_sink: Option<Arc<dyn AssetSink>>,
    per_provider_limit: usize,
    global_limit: usize,
    poll_interval: Duration,
    task_timeout: Duration,
}

impl CreationServiceBuilder {
    /// Wire the unified model invocation service (provider/model resolution +
    /// protocol adapters live there).
    pub fn with_invoke(mut self, invoke: Arc<ModelInvokeService>) -> Self {
        self.invoke = Some(invoke);
        self
    }

    pub fn with_http(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
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
            http: self.http.unwrap_or_default(),
            asset_source: self.asset_source,
            asset_sink: self.asset_sink,
            global_sem: Arc::new(Semaphore::new(self.global_limit)),
            per_provider_limit: self.per_provider_limit,
            provider_sems: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            poll_interval: self.poll_interval,
            task_timeout: self.task_timeout,
        })
    }
}

impl CreationService {
    /// Start a builder over the task repo (invoke layer/sink layered on).
    pub fn builder(repo: Arc<dyn ICreationTaskRepository>) -> CreationServiceBuilder {
        CreationServiceBuilder {
            repo,
            invoke: None,
            http: None,
            asset_source: None,
            asset_sink: None,
            per_provider_limit: DEFAULT_PER_PROVIDER_LIMIT,
            global_limit: DEFAULT_GLOBAL_LIMIT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            task_timeout: DEFAULT_TASK_TIMEOUT,
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

    /// Enqueue a task (`queued`), spawn its worker, and return the queued task.
    /// The worker resolves the provider, loads inputs, runs the adapter, and
    /// drives the state machine to a terminal state asynchronously.
    pub async fn create_task(self: &Arc<Self>, req: NewCreationTask) -> Result<CreationTask, AppError> {
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
        if let Some(invoke) = &self.invoke
            && invoke.provider_repo().find_by_id(&provider_id).await?.is_none()
        {
            return Err(AppError::NotFound(format!(
                "provider {provider_id} not found"
            )));
        }
        let canvas_id = req
            .canvas_id
            .map(WorkshopCanvasId::parse)
            .transpose()
            .map_err(|error| AppError::BadRequest(format!("invalid canvas_id: {error}")))?
            .map(WorkshopCanvasId::into_string);
        let node_id = req
            .node_id
            .map(WorkshopNodeId::parse)
            .transpose()
            .map_err(|error| AppError::BadRequest(format!("invalid node_id: {error}")))?
            .map(WorkshopNodeId::into_string);
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
                Ok(CreationInput { asset_id, role: input.role })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        let params_json = serde_json::to_string(&req.params)
            .map_err(|e| AppError::BadRequest(format!("invalid params json: {e}")))?;
        let creation_task_id = CreationTaskId::new().into_string();
        let now = now_ms();
        let row = self
            .repo
            .create_task(CreateCreationTaskParams {
                creation_task_id: &creation_task_id,
                canvas_id: canvas_id.as_deref(),
                node_id: node_id.as_deref(),
                provider_id: &provider_id,
                model: &req.model,
                capability: capability.as_str(),
                params: &params_json,
                status: TaskStatus::Queued.as_str(),
                submitted_at: now,
            })
            .await?;

        self.spawn(WorkerJob {
            creation_task_id,
            canvas_id,
            node_id,
            provider_id,
            model: req.model,
            capability,
            params: req.params,
            required_artifact_count,
            inputs,
            submitted_at: now,
            remote_task_id: None,
        });

        row.try_into()
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

    pub async fn list_tasks(
        &self,
        canvas_id: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CreationTask>, AppError> {
        let canvas_id = canvas_id
            .map(WorkshopCanvasId::parse)
            .transpose()
            .map_err(|error| AppError::BadRequest(format!("invalid canvas_id: {error}")))?;
        let rows = self
            .repo
            .list_tasks(ListCreationTasksParams {
                canvas_id: canvas_id.as_ref().map(WorkshopCanvasId::as_str),
                status: status.filter(|s| !s.trim().is_empty()),
                limit,
            })
            .await?;
        let rows = self.audit_rows_for_output(rows).await?;
        let mut tasks = rows
            .into_iter()
            .map(CreationTask::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(status) = status.filter(|value| !value.trim().is_empty()) {
            tasks.retain(|task| task.status == status);
        }
        Ok(tasks)
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
                            canvas_id: row.canvas_id,
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
            let handle = parse_job_handle(raw, job.capability);
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
            // Chat text rides the same artifact pipeline as text/plain bytes
            // (the bridge keys its text special case off the MIME prefix).
            TaskResult::Text(text) => vec![ProducedAsset {
                data: ProducedData::Bytes(text.into_bytes()),
                mime: Some(TEXT_MIME.to_string()),
            }],
            // Defensive: no creation capability maps to these result shapes.
            TaskResult::Transcript { .. } | TaskResult::Embeddings(_) => {
                return ExecOutcome::Failed(CreationError::new(
                    "invalid_artifact",
                    "unexpected result type from the invoke layer for a creation task",
                ));
            }
        };
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
        let resp = self
            .http
            .get(url.trim())
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| CreationError::from(net_err(e)))?;
        if !resp.status().is_success() {
            // Any non-2xx on an artifact download is a provider failure (the
            // legacy classification), regardless of the invoke layer's finer
            // status buckets.
            let e = error_from_response(resp).await;
            let mut err = CreationError::provider_error(e.message);
            err.http_status = e.http_status;
            return Err(err);
        }
        let response_content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        let declared_mime = reconcile_mime(mime_hint, response_content_type)?;
        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await.map_err(CreationError::from)?;
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
    if let Some(canvas_id) = &job.canvas_id {
        origin.insert("canvas_id".into(), Value::String(canvas_id.clone()));
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
mod tests {
    use super::*;
    use nomifun_api_types::ModelTask;
    use nomifun_db::{
        CreationTaskRow, DbError, IProviderModelRepository, IProviderRepository, NewProviderModel,
        SqliteCreationTaskRepository, SqliteProviderConnectionRepository,
        SqliteProviderModelRepository, SqliteProviderRepository,
    };
    use nomifun_model_invoke::{AdapterRegistry, InvokeError, ProtocolAdapter, ResolvedCall};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore as TestSemaphore;

    const TEST_KEY: [u8; 32] = [0x42; 32];

    /// Every task the shared "test-model" row declares — the invoke layer's
    /// task-membership gate is exercised by dedicated invoke-layer tests; here
    /// the model is fully tagged so the state-machine tests stay focused.
    const ALL_TEST_TASKS: &str =
        r#"["image_generation","image_edit","video_generation","chat","speech_synthesis"]"#;

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
    /// REAL protocol id (e.g. `"openai.images"`) so the production platform
    /// routing table selects it for the seeded `openai` provider.
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
            JobHandle { adapter_id: self.id.into(), remote_id: "remote-123".into(), poll_state: json!({}) }
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
        async fn create_task(&self, params: CreateCreationTaskParams<'_>) -> Result<CreationTaskRow, DbError> {
            self.inner.create_task(params).await
        }

        async fn get_task(&self, id: &str) -> Result<Option<CreationTaskRow>, DbError> {
            self.inner.get_task(id).await
        }

        async fn list_tasks(&self, params: ListCreationTasksParams<'_>) -> Result<Vec<CreationTaskRow>, DbError> {
            self.inner.list_tasks(params).await
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
        async fn create_task(&self, params: CreateCreationTaskParams<'_>) -> Result<CreationTaskRow, DbError> {
            self.inner.create_task(params).await
        }

        async fn get_task(&self, id: &str) -> Result<Option<CreationTaskRow>, DbError> {
            self.inner.get_task(id).await
        }

        async fn list_tasks(&self, params: ListCreationTasksParams<'_>) -> Result<Vec<CreationTaskRow>, DbError> {
            self.inner.list_tasks(params).await
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
        async fn persist(&self, _asset: PersistAsset) -> Result<String, CreationError> {
            self.count.fetch_add(1, Ordering::SeqCst);
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

    async fn seed_provider(pool: &nomifun_db::SqlitePool, platform: &str) -> String {
        let repo = SqliteProviderRepository::new(pool.clone());
        let encrypted = nomifun_common::encrypt_string("sk-test-key", &TEST_KEY).unwrap();
        let row = repo
            .create(nomifun_db::CreateProviderParams {
                provider_id: None,
                platform,
                name: "Test",
                base_url: "https://api.test.com/v1",
                api_key_encrypted: &encrypted,
                models: "[]",
                enabled: true,
                capabilities: "[]",
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
        row.provider_id
    }

    /// Tag `model` on the provider with `tasks` so the invoke layer's
    /// task-membership gate admits it.
    async fn seed_model_tasks(pool: &nomifun_db::SqlitePool, provider_id: &str, model: &str, tasks: &str) {
        let repo = SqliteProviderModelRepository::new(pool.clone());
        repo.create(
            provider_id,
            &NewProviderModel {
                model,
                enabled: true,
                sort_order: 0,
                tasks,
                traits: "[]",
                protocol: None,
                params: "{}",
                context_limit: None,
                description: None,
                source: "user",
                health: None,
            },
        )
        .await
        .unwrap();
    }

    /// A [`ModelInvokeService`] over the shared pool with ONLY the given
    /// protocol adapters registered (the platform route table then selects
    /// them by their real protocol ids).
    fn invoke_over(pool: &nomifun_db::SqlitePool, adapters: Vec<Arc<dyn ProtocolAdapter>>) -> Arc<ModelInvokeService> {
        Arc::new(ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
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
        _db: nomifun_db::Database,
    }

    async fn harness(adapter: Arc<MockAdapter>, platform: &str) -> Harness {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_id = seed_provider(&pool, platform).await;
        seed_model_tasks(&pool, &provider_id, "test-model", ALL_TEST_TASKS).await;
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let sink = Arc::new(RecordingSink { count: AtomicUsize::new(0) });
        let svc = CreationService::builder(repo)
            .with_invoke(invoke_over(&pool, vec![adapter as Arc<dyn ProtocolAdapter>]))
            .with_asset_source(Arc::new(StaticSource))
            .with_asset_sink(sink.clone())
            .with_poll_interval(Duration::from_millis(10))
            .with_task_timeout(Duration::from_secs(30))
            .build();
        Harness { svc, provider_id, sink, _db: db }
    }

    async fn harness_with_sink_and_repo(
        adapter: Arc<MockAdapter>,
        platform: &str,
        sink: Arc<dyn AssetSink>,
        success_commit_fault: Option<SuccessCommitFault>,
    ) -> (Arc<CreationService>, String, nomifun_db::Database) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_id = seed_provider(&pool, platform).await;
        seed_model_tasks(&pool, &provider_id, "test-model", ALL_TEST_TASKS).await;
        let sqlite_repo: Arc<dyn ICreationTaskRepository> =
            Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let repo: Arc<dyn ICreationTaskRepository> = match success_commit_fault {
            Some(fault) => Arc::new(ScriptedSucceededRepo { inner: sqlite_repo, fault }),
            None => sqlite_repo,
        };
        let svc = CreationService::builder(repo)
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
        repo: &dyn ICreationTaskRepository,
        provider_id: &str,
        capability: &str,
        params: &str,
    ) -> String {
        let creation_task_id = generate_id();
        repo.create_task(CreateCreationTaskParams {
            creation_task_id: &creation_task_id,
            canvas_id: None,
            node_id: None,
            provider_id,
            model: "test-model",
            capability,
            params,
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
        let creation_task_id = create_test_task(svc.repo.as_ref(), provider_id, capability, params).await;
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
            canvas_id: None,
            node_id: None,
            provider_id: provider_id.into(),
            model: "test-model".into(),
            capability: capability.into(),
            params: json!({"prompt": "a cat", "count": 1}),
            inputs: vec![],
        }
    }

    #[tokio::test]
    async fn sync_task_succeeds_and_persists_asset() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;
        let created = h.svc.create_task(new_task(&h.provider_id, "t2i")).await.unwrap();
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
    async fn successful_provider_response_without_artifacts_fails_task() {
        let adapter = MockAdapter::with(
            "openai.images",
            vec![ModelTask::ImageGeneration],
            MockBehavior::DoneEmpty,
        );
        let h = harness(adapter, "openai").await;
        let created = h.svc.create_task(new_task(&h.provider_id, "t2i")).await.unwrap();
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
            let created = h.svc.create_task(new_task(&h.provider_id, "t2i")).await.unwrap();
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
        let created = h.svc.create_task(new_task(&h.provider_id, "t2i")).await.unwrap();
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

        let created = svc.create_task(new_task(&provider_id, "t2i")).await.unwrap();
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

        let created = svc.create_task(new_task(&provider_id, "t2i")).await.unwrap();
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

        let created = svc.create_task(new_task(&provider_id, "t2i")).await.unwrap();
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

        let created = svc.create_task(new_task(&provider_id, "t2i")).await.unwrap();
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
        let created = h.svc.create_task(new_task(&h.provider_id, "t2v")).await.unwrap();
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
        let created = h.svc.create_task(new_task(&h.provider_id, "t2i")).await.unwrap();
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
        let created = h.svc.create_task(new_task(&h.provider_id, "t2v")).await.unwrap();

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
        let provider_id = seed_provider(&pool, "openai").await;
        seed_model_tasks(&pool, &provider_id, "test-model", ALL_TEST_TASKS).await;
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
        let sink = Arc::new(RecordingSink { count: AtomicUsize::new(0) });
        let svc = CreationService::builder(gated.clone())
            .with_invoke(invoke_over(&pool, vec![adapter as Arc<dyn ProtocolAdapter>]))
            .with_asset_source(Arc::new(StaticSource))
            .with_asset_sink(sink)
            .with_poll_interval(Duration::from_millis(10))
            .build();
        let created = svc.create_task(new_task(&provider_id, "t2v")).await.unwrap();

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
        let created = h.svc.create_task(new_task(&h.provider_id, "t2i")).await.unwrap();
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
        assert!(matches!(h.svc.create_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        bad = new_task("  ", "t2i");
        assert!(matches!(h.svc.create_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        bad = new_task(&h.provider_id, "t2i");
        bad.canvas_id = Some("not-a-canvas-id".into());
        assert!(matches!(h.svc.create_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        bad = new_task(&h.provider_id, "t2i");
        bad.node_id = Some("node_1".into());
        assert!(matches!(h.svc.create_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
        bad = new_task(&h.provider_id, "t2i");
        bad.inputs = vec![CreationInput { asset_id: String::new(), role: "reference".into() }];
        assert!(matches!(h.svc.create_task(bad).await.unwrap_err(), AppError::BadRequest(_)));
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
        assert!(matches!(h.svc.list_tasks(Some(""), None, 10).await.unwrap_err(), AppError::BadRequest(_)));
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
                matches!(h.svc.create_task(task).await.unwrap_err(), AppError::BadRequest(_)),
                "invalid image quantity must be rejected before enqueue"
            );
        }
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 0);
        assert!(h.svc.repo.list_all_tasks().await.unwrap().is_empty());

        // The supported ceiling is accepted verbatim (including the `n`
        // alias), and the worker enforces that same prevalidated value.
        let mut task = new_task(&h.provider_id, "t2i");
        task.params = json!({"prompt": "cat", "count": 10, "n": 10});
        let created = h.svc.create_task(task).await.unwrap();
        let done = wait_terminal(&h.svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(done.result_asset_ids.len(), 10);
        assert_eq!(adapter.submit_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn missing_provider_is_rejected_before_task_persistence() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;
        let missing_provider = ProviderId::new().into_string();
        assert!(matches!(
            h.svc.create_task(new_task(&missing_provider, "t2i")).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn boot_reconcile_single_inventory_scan_removes_live_and_missing_task_assets() {
        let adapter = MockAdapter::sync("openai.images");
        let sink = TransactionalTestSink::new(None, None);
        let (svc, provider_id, _db) =
            harness_with_sink_and_repo(adapter, "openai", sink.clone(), None).await;
        let queued_id = create_test_task(svc.repo.as_ref(), &provider_id, "t2i", "{}").await;
        let running_id = create_test_task(svc.repo.as_ref(), &provider_id, "t2i", "{}").await;
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
        let creation_task_id = create_test_task(svc.repo.as_ref(), &provider_id, "t2i", "{}").await;
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
        let creation_task_id = create_test_task(svc.repo.as_ref(), &provider_id, "t2i", "{}").await;
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
    async fn get_and_list_fail_closed_for_short_image_successes_without_rewriting_rows() {
        let h = harness(MockAdapter::sync("openai.images"), "openai").await;

        async fn seed_short_success(
            svc: &CreationService,
            provider_id: &str,
            params: &str,
            result_count: usize,
        ) -> String {
            let id = create_test_task(svc.repo.as_ref(), provider_id, "t2i", params).await;
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

        let list_id = seed_short_success(&h.svc, &h.provider_id, r#"{"n":3}"#, 2).await;
        let list_error = h
            .svc
            .list_tasks(None, Some("succeeded"), 20)
            .await
            .unwrap_err();
        assert!(list_error.to_string().contains("requires at least"));
        assert_eq!(
            h.svc.repo.get_task(&list_id).await.unwrap().unwrap().status,
            "succeeded"
        );
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
            let id = create_test_task(svc.repo.as_ref(), provider_id, "t2i", params).await;
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
        let creation_task_id = create_test_task(svc.repo.as_ref(), &provider_id, "t2i", "{}").await;
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
        let queued_id = create_test_task(repo.as_ref(), &h.provider_id, "t2i", "{}").await;
        let running_id = create_test_task(repo.as_ref(), &h.provider_id, "t2v", "{}").await;
        let resume_id = create_test_task(repo.as_ref(), &h.provider_id, "t2v", "{}").await;

        // (a) a queued leftover → should become failed(interrupted)

        // (b) a running task WITHOUT remote → failed(interrupted)
        repo.update_task(&running_id, UpdateCreationTaskParams { status: Some("running"), ..Default::default() })
            .await
            .unwrap();

        // (c) a running task WITH remote → resumed → succeeded
        repo.update_task(
            &resume_id,
            UpdateCreationTaskParams {
                status: Some("running"),
                remote_task_id: Some(Some("remote-xyz")),
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
        repo
            .create_task(CreateCreationTaskParams {
                creation_task_id: &old_resume_id,
                canvas_id: None,
                node_id: None,
                provider_id: &h.provider_id,
                model: "test-model",
                capability: "t2v",
                params: "{}",
                status: "queued",
                submitted_at: old,
            })
            .await
            .unwrap();
        repo.update_task(
            &old_resume_id,
            UpdateCreationTaskParams {
                status: Some("running"),
                remote_task_id: Some(Some("remote-old")),
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
        let provider_id = seed_provider(db.pool(), "openai").await;
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(db.pool().clone()));
        Box::leak(Box::new(db));
        let svc = CreationService::new(repo);
        let created = svc.create_task(new_task(&provider_id, "t2i")).await.unwrap();
        assert_eq!(created.status, "queued");
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        // No provider repo wired → resolution fails with a config error.
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "config");
    }

    #[test]
    fn build_origin_carries_provenance() {
        let creation_task_id = generate_id();
        let canvas_id = WorkshopCanvasId::new().into_string();
        let node_id = WorkshopNodeId::new().into_string();
        let provider_id = ProviderId::new().into_string();
        let job = WorkerJob {
            creation_task_id: creation_task_id.clone(),
            canvas_id: Some(canvas_id.clone()),
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
        assert_eq!(o["canvas_id"], canvas_id);
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
    fn build_origin_omits_absent_optional_ids_instead_of_writing_null() {
        let job = WorkerJob {
            creation_task_id: CreationTaskId::new().into_string(),
            canvas_id: None,
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
        assert!(!origin.as_object().unwrap().contains_key("canvas_id"));
        assert!(!origin.as_object().unwrap().contains_key("node_id"));
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
    fn cap_to_task_request_maps_every_capability() {
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
                assert_eq!(r.extra, params);
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
        match cap_to_task_request(MediaCapability::Text, &params, vec![]).unwrap() {
            TaskRequest::ChatText(r) => {
                assert_eq!(r.prompt, "a cat");
                assert_eq!(r.system.as_deref(), Some("be brief"));
            }
            _ => panic!("text must map to ChatText"),
        }
        let Err(err) = cap_to_task_request(MediaCapability::V2v, &params, vec![]) else {
            panic!("v2v must be rejected");
        };
        assert_eq!(err.kind, "unsupported_capability");
        let Err(err) = cap_to_task_request(MediaCapability::T2i, &json!({"count": 0}), vec![]) else {
            panic!("count 0 must be rejected");
        };
        assert_eq!(err.kind, "invalid_params");
    }

    // ---- JobHandle persistence compatibility ----

    #[test]
    fn parse_job_handle_accepts_json_and_legacy_bare_id() {
        // Current format: serialized JobHandle JSON round-trips.
        let handle = JobHandle { adapter_id: "openai.videos".into(), remote_id: "vid_1".into(), poll_state: json!({"k": 1}) };
        let raw = serde_json::to_string(&handle).unwrap();
        let parsed = parse_job_handle(&raw, MediaCapability::T2v);
        assert_eq!(parsed.adapter_id, "openai.videos");
        assert_eq!(parsed.remote_id, "vid_1");
        assert_eq!(parsed.poll_state, json!({"k": 1}));

        // Legacy format: a bare provider-side id → wrapped with the
        // capability's legacy protocol (openai.videos — the only async one).
        for cap in [MediaCapability::T2v, MediaCapability::I2v, MediaCapability::V2v] {
            let parsed = parse_job_handle("vid-legacy-42", cap);
            assert_eq!(parsed.adapter_id, "openai.videos");
            assert_eq!(parsed.remote_id, "vid-legacy-42");
            assert_eq!(parsed.poll_state, json!({}));
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
        IProviderModelRepository, IProviderRepository, NewProviderModel, SqliteCreationTaskRepository,
        SqliteProviderConnectionRepository, SqliteProviderModelRepository, SqliteProviderRepository,
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
        let encrypted = nomifun_common::encrypt_string("sk-e2e", &TEST_KEY).unwrap();
        let provider_id = prov_repo
            .create(nomifun_db::CreateProviderParams {
                provider_id: None,
                platform: "openai",
                name: "Mock",
                base_url,
                api_key_encrypted: &encrypted,
                models: "[]",
                enabled: true,
                capabilities: "[]",
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap()
            .provider_id;
        // Tag every test model with the tasks it serves (the invoke layer's
        // task-membership gate refuses untagged models). The gemini text model
        // pins its protocol via the row-level override — the invoke routing
        // table is platform-keyed, and this provider row is "openai".
        let model_repo = SqliteProviderModelRepository::new(pool.clone());
        for (model, tasks, protocol) in [
            ("gpt-image-1", r#"["image_generation","image_edit"]"#, None),
            ("sora-2", r#"["video_generation"]"#, None),
            ("gpt-4o-mini", r#"["chat"]"#, None),
            ("gemini-2.5-flash", r#"["chat"]"#, Some("gemini.generate_text")),
            ("tts-1", r#"["speech_synthesis"]"#, None),
        ] {
            model_repo
                .create(
                    &provider_id,
                    &NewProviderModel {
                        model,
                        enabled: true,
                        sort_order: 0,
                        tasks,
                        traits: "[]",
                        protocol,
                        params: "{}",
                        context_limit: None,
                        description: None,
                        source: "user",
                        health: None,
                    },
                )
                .await
                .unwrap();
        }
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(pool.clone()));
        let invoke = Arc::new(ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
        ));
        let sink = Arc::new(CountingSink { count: AtomicUsize::new(0), persisted: std::sync::Mutex::new(Vec::new()) });
        let svc = CreationService::builder(repo)
            .with_invoke(invoke)
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
            canvas_id: None,
            node_id: None,
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
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": encoded}]})))
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_task(t2i(&provider_id)).await.unwrap();
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
            .and(path("/v1/images/generations"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"b64_json": encoded}]})),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let mut request = t2i(&provider_id);
        request.params["count"] = json!(4);
        let created = svc.create_task(request).await.unwrap();
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
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"url": "  "}]})))
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let created = svc.create_task(t2i(&provider_id)).await.unwrap();
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
            .and(path("/v1/images/generations"))
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
        let created = svc.create_task(t2i(&provider_id)).await.unwrap();
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
            .and(path("/v1/images/generations"))
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
        let created = svc.create_task(t2i(&provider_id)).await.unwrap();
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
            .and(path("/v1/images/generations"))
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
        let created = svc.create_task(t2i(&provider_id)).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "invalid_artifact");
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn openai_images_propagates_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let (svc, provider_id, _sink, _db) = build(&server.uri()).await;
        let created = svc.create_task(t2i(&provider_id)).await.unwrap();
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
            .and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "queued"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "completed"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_1/content"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .set_body_bytes(valid_mp4()),
            )
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            canvas_id: None,
            node_id: None,
            provider_id: provider_id.clone(),
            model: "sora-2".into(),
            capability: "t2v".into(),
            params: json!({"prompt": "a wave", "seconds": 4}),
            inputs: vec![],
        };
        let created = svc.create_task(task).await.unwrap();
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
            .and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "v1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "v1", "status": "failed", "error": {"message": "moderation blocked"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            canvas_id: None,
            node_id: None,
            provider_id: provider_id.clone(),
            model: "sora-2".into(),
            capability: "t2v".into(),
            params: json!({"prompt": "a wave", "seconds": 4}),
            inputs: vec![],
        };
        let created = svc.create_task(task).await.unwrap();
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
    async fn openai_chat_text_end_to_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "hello from the model"}}]
            })))
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            canvas_id: None,
            node_id: None,
            provider_id: provider_id.clone(),
            model: "gpt-4o-mini".into(),
            capability: "text".into(),
            params: json!({"prompt": "say hi"}),
            inputs: vec![],
        };
        let created = svc.create_task(task).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        assert_eq!(sink.count.load(Ordering::SeqCst), 1);
        let persisted = sink.persisted.lock().unwrap();
        assert_eq!(persisted.len(), 1);
        assert!(persisted[0].0.starts_with("text/plain"), "mime={}", persisted[0].0);
        assert_eq!(String::from_utf8_lossy(&persisted[0].1), "hello from the model");
    }

    #[tokio::test]
    async fn gemini_text_end_to_end() {
        let server = MockServer::start().await;
        // The seeded provider_models row pins protocol "gemini.generate_text"
        // (invoke routing is platform-keyed; this provider row is "openai").
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [{"text": "gemini says "}, {"text": "hi"}]}}]
            })))
            .mount(&server)
            .await;

        let (svc, provider_id, sink, _db) = build(&server.uri()).await;
        let task = NewCreationTask {
            canvas_id: None,
            node_id: None,
            provider_id: provider_id.clone(),
            model: "gemini-2.5-flash".into(),
            capability: "text".into(),
            params: json!({"prompt": "greet me"}),
            inputs: vec![],
        };
        let created = svc.create_task(task).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "succeeded", "error={:?}", done.error);
        let persisted = sink.persisted.lock().unwrap();
        assert_eq!(persisted.len(), 1);
        assert!(persisted[0].0.starts_with("text/plain"), "mime={}", persisted[0].0);
        assert_eq!(String::from_utf8_lossy(&persisted[0].1), "gemini says hi");
    }

    #[tokio::test]
    async fn tts_end_to_end_produces_audio_artifact() {
        // Tts is now routable: capability "tts" maps to SpeechSynthesis →
        // openai.audio_speech → POST /v1/audio/speech returning audio bytes.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
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
            canvas_id: None,
            node_id: None,
            provider_id: provider_id.clone(),
            model: "tts-1".into(),
            capability: "tts".into(),
            params: json!({"prompt": "hello there", "voice": "alloy"}),
            inputs: vec![],
        };
        let created = svc.create_task(task).await.unwrap();
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
        let created = svc.create_task(request).await.unwrap();
        let done = wait_terminal(&svc, &created.creation_task_id).await;
        assert_eq!(done.status, "failed");
        assert_eq!(done.error.as_ref().unwrap()["kind"], "unsupported_capability");
        assert_eq!(sink.count.load(Ordering::SeqCst), 0);
        assert!(server.received_requests().await.unwrap().is_empty(), "gate must fire before the wire");
    }
}
