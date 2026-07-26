//! Shared Browser Platform tool dispatcher.
//!
//! This is deliberately smaller than [`crate::BrowserTool`]: it owns no
//! browser engine, Chromium process, profile, or caller identity.  It accepts
//! only an already-bound [`BrowserLaneClient`] and therefore gives the native
//! Agent, Gateway, and ACP/stdio surfaces one owner-scoped Lane contract.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nomi_types::tool::{ToolImage, ToolResult};
use nomifun_browser_platform::{
    BrowserIdentityMode, BrowserLaneClient, BrowserLaneId, BrowserLaneSnapshot,
    BrowserOperation, BrowserOperationKind, BrowserOperationResult, BrowserPlatformError,
    CloseResult, LaneLifecycleState, OpenLaneOutcome,
};
use serde_json::{Value, json};

use crate::OUT_OF_BAND_CONFIRMED_KEY;

const MAX_CRAWL_CONCURRENCY: usize = 8;
const MAX_CRAWL_URLS: usize = 64;
const CRAWL_OPERATION_TIMEOUT: Duration = Duration::from_secs(45);
const CRAWL_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const INVALID_CRAWL_REQUEST_CODE: &str = "invalid_browser_request";
static MANAGED_LANE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Identity selection is trusted host policy, never model input. Keep this
/// separate from owner metadata so callers receive the stable
/// `invalid_browser_request` contract rather than an owner-spoofing error.
const MODEL_IDENTITY_INPUT_FIELDS: &[&str] = &[
    "identity",
    "identity_mode",
    "authenticated",
    "auth_identity",
    "profile",
    "account",
];

/// Fields whose authority belongs to the main process.  A caller may select an
/// owner-scoped `lane_id`, but it may never construct or override identity,
/// target ownership, epochs, cancellation, or resource routing.
const TRUSTED_OWNER_INPUT_FIELDS: &[&str] = &[
    "caller",
    "caller_identity",
    "user_id",
    "conversation_id",
    "runtime_instance_id",
    "agent_id",
    "companion_id",
    "execution_id",
    "step_id",
    "attempt_id",
    "remote_connection_id",
    "owner_lease_id",
    "capability_expires_at_ms",
    "allowed_operations",
    "identity_generation",
    "browser_epoch",
    "target_id",
    "frame_id",
    "ref_generation",
    "cancellation_id",
    "workspace_hint",
    "surface",
    "browser_surface",
    "lane_key",
];

#[async_trait]
pub(crate) trait BrowserLaneClientPort: Send + Sync {
    async fn open(
        &self,
        lane_name: Option<&str>,
        identity_mode: BrowserIdentityMode,
        workspace_hint: Option<String>,
    ) -> Result<OpenLaneOutcome, BrowserPlatformError>;

    async fn execute(
        &self,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError>;

    async fn list(&self) -> Result<Vec<BrowserLaneSnapshot>, BrowserPlatformError>;

    async fn status(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError>;

    async fn close(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<CloseResult, BrowserPlatformError>;

    async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError>;
}

#[async_trait]
impl BrowserLaneClientPort for BrowserLaneClient {
    async fn open(
        &self,
        lane_name: Option<&str>,
        identity_mode: BrowserIdentityMode,
        workspace_hint: Option<String>,
    ) -> Result<OpenLaneOutcome, BrowserPlatformError> {
        BrowserLaneClient::open(self, lane_name, identity_mode, workspace_hint).await
    }

    async fn execute(
        &self,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        BrowserLaneClient::execute(self, lane_id, operation).await
    }

    async fn list(&self) -> Result<Vec<BrowserLaneSnapshot>, BrowserPlatformError> {
        BrowserLaneClient::list(self).await
    }

    async fn status(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        BrowserLaneClient::status(self, lane_id).await
    }

    async fn close(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        BrowserLaneClient::close(self, lane_id).await
    }

    async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError> {
        BrowserLaneClient::close_all(self).await
    }
}

/// A clone-cheap, owner-bound Browser Platform dispatcher.
#[derive(Clone)]
pub struct ManagedBrowserFacade {
    client: Arc<dyn BrowserLaneClientPort>,
    workspace_dir: Option<PathBuf>,
}

impl ManagedBrowserFacade {
    pub fn new(client: BrowserLaneClient, workspace_dir: Option<PathBuf>) -> Self {
        Self {
            client: Arc::new(client),
            workspace_dir,
        }
    }

    /// Dispatch an existing action or one of the Lane-management actions.
    ///
    /// Existing actions default to the caller's `default` Lane and accept an
    /// optional owner-scoped `lane_id`.  The bound Hub client performs the
    /// definitive authorization check, so a Lane handle from another owner
    /// fails closed.
    pub async fn execute(&self, action: &str, input: &Value) -> ToolResult {
        if let Some(field) = first_model_identity_field(input) {
            return invalid_browser_request(format!(
                "Browser input field `{field}` selects trusted host identity policy and is not accepted. \
                 Interactive Lanes use Primary identity and browser_crawl_many uses Anonymous identity."
            ));
        }

        if let Some(field) = first_trusted_owner_field(input) {
            return ToolResult::error(pretty_json(&json!({
                "ok": false,
                "error": {
                    "code": "invalid_caller_identity",
                    "message": format!(
                        "Browser input field `{field}` is trusted host metadata and is not accepted."
                    ),
                    "retryable": false,
                    "next_action": "Remove caller, target, epoch, and resource-routing fields.",
                }
            })));
        }

        let canonical = match action {
            "open" => "browser_open",
            "fork" => "browser_fork",
            "list" => "browser_list",
            "status" => "browser_status",
            "close" => "browser_close",
            "close_all" => "browser_close_all",
            "crawl_many" => "browser_crawl_many",
            other => other,
        };

        match canonical {
            "browser_open" => self.open(input, false).await,
            "browser_fork" => self.open(input, true).await,
            "browser_list" => self.list().await,
            "browser_status" => self.status(input).await,
            "browser_close" => self.close(input).await,
            "browser_close_all" => self.close_all().await,
            "browser_crawl_many" => self.crawl_many(input).await,
            existing if is_existing_browser_action(existing) => {
                self.execute_existing(existing, input).await
            }
            unsupported => ToolResult::error(pretty_json(&json!({
                "ok": false,
                "error": {
                    "code": "operation_not_allowed",
                    "message": format!("Unsupported Browser action `{unsupported}`."),
                    "retryable": false,
                    "next_action": "Use a registered Browser action or Lane-management tool.",
                }
            }))),
        }
    }

    async fn open(&self, input: &Value, fork: bool) -> ToolResult {
        let generated_name;
        let lane_name = match input.get("lane_name").and_then(Value::as_str) {
            Some(name) => Some(name),
            None if fork => {
                let sequence = MANAGED_LANE_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
                generated_name = format!("fork-{sequence}");
                Some(generated_name.as_str())
            }
            None => None,
        };
        let workspace_hint = self
            .workspace_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        match self
            .client
            .open(lane_name, BrowserIdentityMode::Primary, workspace_hint)
            .await
        {
            Ok(outcome) => {
                let lane = outcome.lane();
                ToolResult::text(pretty_json(&json!({
                    "ok": true,
                    "action": if fork { "browser_fork" } else { "browser_open" },
                    "lane": public_lane_json(lane),
                    "queued": matches!(outcome, OpenLaneOutcome::Queued { .. }),
                    "next_action": lane_next_action(lane),
                })))
            }
            Err(error) => platform_error_result("Opening a browser Lane failed", error),
        }
    }

    async fn list(&self) -> ToolResult {
        match self.client.list().await {
            Ok(lanes) => ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": "browser_list",
                "lanes": lanes.iter().map(public_lane_json).collect::<Vec<_>>(),
            }))),
            Err(error) => platform_error_result("Listing browser Lanes failed", error),
        }
    }

    async fn status(&self, input: &Value) -> ToolResult {
        let lane_id = match self.resolve_lane_id(input, false).await {
            Ok(Some(lane_id)) => lane_id,
            Ok(None) => {
                return invalid_browser_request(
                    "No default browser Lane exists. Run `browser_open` first.",
                );
            }
            Err(error) => return platform_error_result("Resolving the browser Lane failed", error),
        };
        match self.client.status(&lane_id).await {
            Ok(lane) => ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": "browser_status",
                "lane": public_lane_json(&lane),
                "next_action": lane_next_action(&lane),
            }))),
            Err(error) => platform_error_result("Reading browser Lane status failed", error),
        }
    }

    async fn close(&self, input: &Value) -> ToolResult {
        let lane_id = match self.resolve_lane_id(input, false).await {
            Ok(lane_id) => lane_id,
            Err(error) => return platform_error_result("Resolving the browser Lane failed", error),
        };
        let Some(lane_id) = lane_id else {
            return ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": "browser_close",
                "closed": 0,
                "already_closed": true,
            })));
        };
        match self.client.close(&lane_id).await {
            Ok(result) => ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": "browser_close",
                "lane_id": lane_id.as_str(),
                "closed": result.closed,
                "already_closed": result.already_closed,
            }))),
            Err(error) => platform_error_result("Closing the browser Lane failed", error),
        }
    }

    async fn close_all(&self) -> ToolResult {
        match self.client.close_all().await {
            Ok(result) => ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": "browser_close_all",
                "closed": result.closed,
                "already_closed": result.already_closed,
            }))),
            Err(error) => platform_error_result("Closing browser Lanes failed", error),
        }
    }

    async fn execute_existing(&self, action: &str, input: &Value) -> ToolResult {
        let lane = match self.resolve_running_lane(input).await {
            Ok(lane) => lane,
            Err(error) => return platform_error_result("Resolving the browser Lane failed", error),
        };
        if lane.lifecycle_state != LaneLifecycleState::Running {
            return ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": action,
                "dispatched": false,
                "lane": public_lane_json(&lane),
                "next_action": lane_next_action(&lane),
            })));
        }

        let operation = BrowserOperation {
            kind: operation_kind(action),
            action: action.to_owned(),
            input: sanitize_operation_input(input),
            expected_browser_epoch: input
                .get("expected_browser_epoch")
                .or_else(|| input.get("browser_epoch"))
                .and_then(Value::as_u64),
            target_id: None,
            frame_id: None,
            // `f<seq>e<n>` embeds a frame sequence, not the observation
            // generation. Bind ref-bearing operations to the Hub's
            // authoritative lane snapshot instead of parsing the ref text.
            ref_generation: input
                .get("ref")
                .and_then(Value::as_str)
                .is_some()
                .then_some(lane.ref_generation),
            may_modify_identity: action_may_modify_identity(action, input),
        };
        match self.client.execute(&lane.lane_id, operation).await {
            Ok(result) => {
                let latest = self.client.status(&lane.lane_id).await.ok();
                operation_result(action, &lane.lane_id, result, latest.as_ref())
            }
            Err(error) => platform_error_result("Browser Lane operation failed", error),
        }
    }

    async fn resolve_running_lane(
        &self,
        input: &Value,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        if let Some(lane_id) = managed_lane_id(input)? {
            return self.client.status(&lane_id).await;
        }
        let workspace_hint = self
            .workspace_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        self.client
            .open(None, BrowserIdentityMode::Primary, workspace_hint)
            .await
            .map(|outcome| outcome.lane().clone())
    }

    async fn resolve_lane_id(
        &self,
        input: &Value,
        open_default: bool,
    ) -> Result<Option<BrowserLaneId>, BrowserPlatformError> {
        if let Some(lane_id) = managed_lane_id(input)? {
            // Authorization happens here before the handle is used.
            self.client.status(&lane_id).await?;
            return Ok(Some(lane_id));
        }
        if let Some(lane) = self
            .client
            .list()
            .await?
            .into_iter()
            .find(|lane| lane.lane_key.lane_name == "default")
        {
            return Ok(Some(lane.lane_id));
        }
        if !open_default {
            return Ok(None);
        }
        self.resolve_running_lane(input)
            .await
            .map(|lane| Some(lane.lane_id))
    }

    async fn crawl_many(&self, input: &Value) -> ToolResult {
        execute_crawl_many_input(
            Arc::clone(&self.client),
            input,
            self.workspace_dir.as_deref(),
        )
        .await
    }
}

/// Convert the shared dispatcher result to the loopback/Gateway envelope.
pub fn managed_result_envelope(result: ToolResult) -> Value {
    let parsed = serde_json::from_str::<Value>(&result.content)
        .unwrap_or_else(|_| Value::String(result.content));
    if result.is_error {
        return json!({ "error": parsed });
    }
    let images = result
        .images
        .iter()
        .map(|image| {
            json!({
                "mime_type": image.media_type,
                "data": image.data,
            })
        })
        .collect::<Vec<_>>();
    if images.is_empty() {
        json!({ "result": parsed })
    } else {
        json!({
            "result": parsed,
            "_mcp_images": images,
        })
    }
}

pub(crate) struct ManagedCrawlRequest {
    urls: Vec<String>,
    requested_concurrency: usize,
    auto_concurrency: bool,
    identity_mode: BrowserIdentityMode,
    schema: Option<Value>,
    workspace_hint: Option<String>,
}

struct CrawlWorkerPlan {
    lane: BrowserLaneSnapshot,
    items: Vec<(usize, String)>,
    recommended_concurrency: usize,
}

enum CrawlLaneOpenFailure {
    Unavailable(BrowserLaneSnapshot),
    Rejected {
        lane: BrowserLaneSnapshot,
        error: BrowserPlatformError,
    },
    Open(BrowserPlatformError),
}

/// Owns only Lane names that were absent from the preflight inventory and
/// concrete Lane IDs returned by this batch's successful opens. A normal error
/// disarms a pending name without closing it; cancellation during `open` keeps
/// the pending name so Drop can resolve the partially-created Lane.
struct CrawlBatchCleanup {
    client: Arc<dyn BrowserLaneClientPort>,
    owned_lanes: HashMap<BrowserLaneId, String>,
    pending_names: HashSet<String>,
}

impl CrawlBatchCleanup {
    fn new(client: Arc<dyn BrowserLaneClientPort>) -> Self {
        Self {
            client,
            owned_lanes: HashMap::new(),
            pending_names: HashSet::new(),
        }
    }

    fn track_pending_name(&mut self, lane_name: String) {
        self.pending_names.insert(lane_name);
    }

    fn abandon_pending_name(&mut self, lane_name: &str) {
        self.pending_names.remove(lane_name);
    }

    fn track_owned_lane(&mut self, lane: &BrowserLaneSnapshot) {
        self.pending_names.remove(&lane.lane_key.lane_name);
        self.owned_lanes
            .insert(lane.lane_id.clone(), lane.lane_key.lane_name.clone());
    }

    async fn close_selected(
        &mut self,
        lane_ids: impl IntoIterator<Item = BrowserLaneId>,
    ) -> HashMap<String, Result<CloseResult, BrowserPlatformError>> {
        let ids = lane_ids
            .into_iter()
            .filter(|lane_id| self.owned_lanes.contains_key(lane_id))
            .collect::<Vec<_>>();
        let results = close_lane_ids_until(
            Arc::clone(&self.client),
            ids,
            tokio::time::Instant::now() + CRAWL_CLEANUP_TIMEOUT,
        )
        .await;
        for (lane_id, result) in &results {
            if result.is_ok() {
                self.owned_lanes.remove(lane_id);
            }
        }
        results
            .into_iter()
            .map(|(lane_id, result)| (lane_id.as_str().to_owned(), result))
            .collect()
    }

    async fn close_all_owned(
        &mut self,
    ) -> HashMap<String, Result<CloseResult, BrowserPlatformError>> {
        self.close_selected(self.owned_lanes.keys().cloned().collect::<Vec<_>>())
            .await
    }
}

impl Drop for CrawlBatchCleanup {
    fn drop(&mut self) {
        if self.owned_lanes.is_empty() && self.pending_names.is_empty() {
            return;
        }
        let client = Arc::clone(&self.client);
        let lane_ids = std::mem::take(&mut self.owned_lanes)
            .into_keys()
            .collect::<Vec<_>>();
        let pending_names = std::mem::take(&mut self.pending_names);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                lane_count = lane_ids.len(),
                lane_name_count = pending_names.len(),
                "managed crawl batch dropped outside Tokio; Lane cleanup could not be scheduled"
            );
            return;
        };
        runtime.spawn(async move {
            cleanup_dropped_crawl_batch(client, lane_ids, pending_names).await;
        });
    }
}

pub(crate) async fn execute_crawl_many_input(
    client: Arc<dyn BrowserLaneClientPort>,
    input: &Value,
    workspace_dir: Option<&std::path::Path>,
) -> ToolResult {
    let request = match parse_crawl_request(input, workspace_dir) {
        Ok(request) => request,
        Err(message) => return invalid_browser_request(message),
    };
    execute_managed_crawl_many(client, request).await
}

async fn execute_managed_crawl_many(
    client: Arc<dyn BrowserLaneClientPort>,
    request: ManagedCrawlRequest,
) -> ToolResult {
    let lane_names =
        match unused_crawl_lane_names(client.as_ref(), request.requested_concurrency).await {
            Ok(names) => names,
            Err(error) => {
                return crawl_batch_result(
                    &request,
                    0,
                    request
                        .urls
                        .iter()
                        .cloned()
                        .map(|url| {
                            crawl_error_result(
                                url,
                                None,
                                request.identity_mode,
                                request.requested_concurrency,
                                public_platform_error_json(&error),
                            )
                        })
                        .collect(),
                );
            }
        };
    let mut cleanup = CrawlBatchCleanup::new(Arc::clone(&client));
    let mut running_lanes = Vec::new();
    let mut unavailable_lanes = Vec::new();
    let mut rejected_lanes = Vec::new();
    let mut open_errors = Vec::new();
    let mut open_limit = request.requested_concurrency;
    let mut worker_index = 0;
    while worker_index < open_limit {
        let lane_name = lane_names[worker_index].clone();
        cleanup.track_pending_name(lane_name.clone());
        match client
            .open(
                Some(&lane_name),
                request.identity_mode,
                request.workspace_hint.clone(),
            )
            .await
        {
            Err(error) => {
                // The Hub rejected this open, so this batch did not acquire the
                // Lane. Never close a same-name Lane that may belong to the caller.
                cleanup.abandon_pending_name(&lane_name);
                open_errors.push((worker_index, error));
            }
            Ok(outcome) => {
                let lane = outcome.lane().clone();
                cleanup.track_owned_lane(&lane);
                if lane.identity_mode != request.identity_mode {
                    let error =
                        BrowserPlatformError::new(
                            nomifun_browser_platform::BrowserErrorCode::OperationNotAllowed,
                            "The crawl Lane resolved to a different identity mode.",
                            false,
                            "Retry with a fresh crawl batch; do not reuse this Lane.",
                        )
                        .for_lane(lane.lane_id.clone());
                    rejected_lanes.push((worker_index, lane, error));
                } else if lane.lifecycle_state == LaneLifecycleState::Running {
                    running_lanes.push((worker_index, lane));
                } else {
                    if request.auto_concurrency {
                        if let Some(recommended) = lane
                            .queue
                            .as_ref()
                            .map(|queue| queue.recommended_concurrency)
                        {
                            open_limit = open_limit.min(
                                recommended
                                    .max(running_lanes.len())
                                    .max(1)
                                    .min(request.requested_concurrency),
                            );
                        }
                    }
                    unavailable_lanes.push((worker_index, lane));
                }
            }
        }
        worker_index += 1;
    }

    let blocked_ids = unavailable_lanes
        .iter()
        .map(|(_, lane)| lane.lane_id.clone())
        .chain(
            rejected_lanes
                .iter()
                .map(|(_, lane, _)| lane.lane_id.clone()),
        )
        .collect::<Vec<_>>();
    let blocked_cleanup = cleanup.close_selected(blocked_ids).await;

    if running_lanes.is_empty() {
        let cleanup_results = if cleanup.owned_lanes.is_empty() {
            blocked_cleanup
        } else {
            let mut cleanup_results = blocked_cleanup;
            cleanup_results.extend(cleanup.close_all_owned().await);
            cleanup_results
        };
        let mut terminal_lanes = HashMap::new();
        for (worker_index, lane) in unavailable_lanes {
            terminal_lanes.insert(worker_index, CrawlLaneOpenFailure::Unavailable(lane));
        }
        for (worker_index, lane, error) in rejected_lanes {
            terminal_lanes.insert(
                worker_index,
                CrawlLaneOpenFailure::Rejected { lane, error },
            );
        }
        for (worker_index, error) in open_errors {
            terminal_lanes.insert(worker_index, CrawlLaneOpenFailure::Open(error));
        }
        let fallback_error = BrowserPlatformError::new(
            nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
            "No crawl Lane reached the running state.",
            true,
            "Retry later or lower crawl concurrency.",
        );
        let results = request
            .urls
            .iter()
            .enumerate()
            .map(|(index, url)| {
                let worker_index = index % request.requested_concurrency;
                match terminal_lanes.get(&worker_index) {
                    Some(CrawlLaneOpenFailure::Unavailable(lane)) => {
                        let mut result = crawl_lane_unavailable_result(
                            url,
                            lane,
                            lane.queue
                                .as_ref()
                                .map(|queue| queue.recommended_concurrency)
                                .unwrap_or(request.requested_concurrency),
                        );
                        if let Some(close) = cleanup_results.get(lane.lane_id.as_str()) {
                            attach_crawl_cleanup(&mut result, close);
                        }
                        result
                    }
                    Some(CrawlLaneOpenFailure::Rejected { lane, error }) => {
                        let mut result = crawl_error_result(
                            url.clone(),
                            Some(lane),
                            request.identity_mode,
                            request.requested_concurrency,
                            public_platform_error_json(error),
                        );
                        result["dispatched"] = Value::Bool(false);
                        if let Some(close) = cleanup_results.get(lane.lane_id.as_str()) {
                            attach_crawl_cleanup(&mut result, close);
                        }
                        result
                    }
                    Some(CrawlLaneOpenFailure::Open(error)) => crawl_error_result(
                        url.clone(),
                        None,
                        request.identity_mode,
                        request.requested_concurrency,
                        public_platform_error_json(error),
                    ),
                    None => crawl_error_result(
                        url.clone(),
                        None,
                        request.identity_mode,
                        request.requested_concurrency,
                        public_platform_error_json(&fallback_error),
                    ),
                }
            })
            .collect();
        return crawl_batch_result(&request, 0, results);
    }

    let effective_concurrency = running_lanes.len();
    let mut partitions = vec![Vec::<(usize, String)>::new(); effective_concurrency];
    for (index, url) in request.urls.iter().cloned().enumerate() {
        partitions[index % effective_concurrency].push((index, url));
    }
    running_lanes.sort_by_key(|(worker_index, _)| *worker_index);
    let mut ordered_results = vec![None; request.urls.len()];
    let mut workers = tokio::task::JoinSet::new();
    let mut worker_plans = HashMap::new();
    for ((_, lane), items) in running_lanes.into_iter().zip(partitions) {
        let plan = CrawlWorkerPlan {
            lane: lane.clone(),
            items: items.clone(),
            recommended_concurrency: effective_concurrency,
        };
        let abort_handle = workers.spawn(run_crawl_worker(
            Arc::clone(&client),
            lane,
            items,
            request.schema.clone(),
            effective_concurrency,
        ));
        worker_plans.insert(abort_handle.id(), plan);
    }

    while let Some(joined) = workers.join_next_with_id().await {
        match joined {
            Ok((task_id, worker_results)) => {
                let Some(plan) = worker_plans.remove(&task_id) else {
                    continue;
                };
                let mut by_index = worker_results.into_iter().collect::<HashMap<_, _>>();
                for (index, url) in &plan.items {
                    ordered_results[*index] = Some(by_index.remove(index).unwrap_or_else(|| {
                        crawl_worker_terminal_failure(
                            url,
                            &plan.lane,
                            plan.recommended_concurrency,
                            "worker_incomplete",
                        )
                    }));
                }
            }
            Err(join_error) => {
                let Some(plan) = worker_plans.remove(&join_error.id()) else {
                    continue;
                };
                let cause = if join_error.is_panic() {
                    "worker_panicked"
                } else if join_error.is_cancelled() {
                    "worker_cancelled"
                } else {
                    "worker_join_failed"
                };
                // A worker Lane is the ownership and recovery unit.  If the
                // worker terminates unexpectedly, every URL assigned to that
                // Lane is terminally failed, including URLs it may have
                // completed just before the panic.  This prevents a partial
                // batch from being reported as successful after its Lane has
                // become unusable and keeps assignment deterministic.
                for (index, url) in &plan.items {
                    ordered_results[*index] = Some(crawl_worker_terminal_failure(
                        url,
                        &plan.lane,
                        effective_concurrency,
                        cause,
                    ));
                }
            }
        }
    }

    // `JoinSet` should only become empty after every task produced a join
    // outcome. Keep the one-input/one-result contract even if a future
    // runtime invariant says otherwise. A missing result cannot be attributed
    // to a specific Lane, so return a metadata-safe batch error.
    for (index, url) in request.urls.iter().enumerate() {
        if ordered_results[index].is_none() {
            ordered_results[index] = Some(crawl_error_result(
                url.clone(),
                None,
                request.identity_mode,
                effective_concurrency,
                json!({
                    "code": "crawl_result_missing",
                    "message": "The crawl batch did not produce a terminal result for this URL.",
                    "retryable": true,
                    "next_action": "Retry this URL in a new bounded crawl batch.",
                }),
            ));
        }
    }
    let mut results = ordered_results
        .into_iter()
        .map(|result| result.expect("crawl result filled"))
        .collect::<Vec<_>>();

    let cleanup_results = cleanup.close_all_owned().await;
    for result in &mut results {
        if let Some(lane_id) = result.get("lane_id").and_then(Value::as_str) {
            if let Some(close) = cleanup_results.get(lane_id) {
                attach_crawl_cleanup(result, close);
            }
        }
    }
    crawl_batch_result(&request, effective_concurrency, results)
}

async fn unused_crawl_lane_names(
    client: &dyn BrowserLaneClientPort,
    count: usize,
) -> Result<Vec<String>, BrowserPlatformError> {
    let existing = client
        .list()
        .await?
        .into_iter()
        .map(|lane| lane.lane_key.lane_name)
        .collect::<HashSet<_>>();
    for _ in 0..8 {
        let candidates = new_crawl_lane_names(count);
        if candidates.iter().all(|name| !existing.contains(name)) {
            return Ok(candidates);
        }
    }
    Err(BrowserPlatformError::new(
        nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
        "Could not allocate a private crawl Lane namespace.",
        true,
        "Retry the crawl batch.",
    ))
}

fn new_crawl_lane_names(count: usize) -> Vec<String> {
    let batch_id = BrowserLaneId::new();
    let compact = batch_id
        .as_str()
        .bytes()
        .filter(|byte| byte.is_ascii_hexdigit())
        .map(char::from)
        .collect::<String>();
    let nonce = &compact[compact.len().saturating_sub(24)..];
    (1..=count)
        .map(|worker| format!("_nc-{nonce}-{worker}"))
        .collect()
}

async fn run_crawl_worker(
    client: Arc<dyn BrowserLaneClientPort>,
    lane: BrowserLaneSnapshot,
    items: Vec<(usize, String)>,
    schema: Option<Value>,
    recommended_concurrency: usize,
) -> Vec<(usize, Value)> {
    let mut results = Vec::with_capacity(items.len());
    for (index, url) in items {
        let result = run_crawl_item(
            Arc::clone(&client),
            &lane,
            url,
            schema.as_ref(),
            recommended_concurrency,
        )
        .await;
        results.push((index, result));
    }
    results
}

async fn run_crawl_item(
    client: Arc<dyn BrowserLaneClientPort>,
    lane: &BrowserLaneSnapshot,
    url: String,
    schema: Option<&Value>,
    recommended_concurrency: usize,
) -> Value {
    let navigate = BrowserOperation {
        kind: BrowserOperationKind::Crawl,
        action: "navigate".to_owned(),
        input: json!({ "action": "navigate", "url": url }),
        expected_browser_epoch: Some(lane.browser_epoch),
        target_id: None,
        frame_id: None,
        ref_generation: None,
        // Authenticated replicas are intentionally usable for read-oriented
        // authenticated crawling. Incidental replica-local cookie/storage
        // changes are not merged back into the canonical identity.
        may_modify_identity: false,
    };
    let navigation = match tokio::time::timeout(
        CRAWL_OPERATION_TIMEOUT,
        client.execute(&lane.lane_id, navigate),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            return crawl_error_result(
                url,
                Some(lane),
                lane.identity_mode,
                recommended_concurrency,
                public_platform_error_json(&error),
            );
        }
        Err(_) => {
            return crawl_error_result(
                url,
                Some(lane),
                lane.identity_mode,
                recommended_concurrency,
                json!({
                    "code": "crawl_timeout",
                    "message": "The crawl navigation exceeded its operation timeout.",
                    "retryable": true,
                    "next_action": "Retry this URL or lower crawl concurrency.",
                }),
            );
        }
    };

    let (content_action, content_input) = match schema {
        Some(schema) => ("extract", json!({ "action": "extract", "schema": schema })),
        None => ("get_page_text", json!({ "action": "get_page_text" })),
    };
    let content = BrowserOperation {
        kind: BrowserOperationKind::Crawl,
        action: content_action.to_owned(),
        input: content_input,
        expected_browser_epoch: Some(lane.browser_epoch),
        target_id: None,
        frame_id: None,
        ref_generation: None,
        may_modify_identity: false,
    };
    let extracted = match tokio::time::timeout(
        CRAWL_OPERATION_TIMEOUT,
        client.execute(&lane.lane_id, content),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            let mut value = crawl_error_result(
                url,
                Some(lane),
                lane.identity_mode,
                recommended_concurrency,
                public_platform_error_json(&error),
            );
            value["navigation"] = navigation.output;
            return value;
        }
        Err(_) => {
            let mut value = crawl_error_result(
                url,
                Some(lane),
                lane.identity_mode,
                recommended_concurrency,
                json!({
                    "code": "crawl_timeout",
                    "message": "The crawl extraction exceeded its operation timeout.",
                    "retryable": true,
                    "next_action": "Retry this URL or request a smaller extraction.",
                }),
            );
            value["navigation"] = navigation.output;
            return value;
        }
    };
    let latest = client
        .status(&lane.lane_id)
        .await
        .unwrap_or_else(|_| lane.clone());
    crawl_success_result(
        url,
        &latest,
        recommended_concurrency,
        navigation.output,
        extracted.output,
    )
}

fn crawl_batch_result(
    request: &ManagedCrawlRequest,
    effective_concurrency: usize,
    results: Vec<Value>,
) -> ToolResult {
    ToolResult::text(pretty_json(&json!({
        "ok": results.iter().all(|result| {
            result.get("ok").and_then(Value::as_bool).unwrap_or(false)
        }),
        "action": "browser_crawl_many",
        "identity_mode": request.identity_mode,
        "requested": request.urls.len(),
        "concurrency": effective_concurrency,
        "results": results,
    })))
}

fn crawl_success_result(
    url: String,
    lane: &BrowserLaneSnapshot,
    recommended_concurrency: usize,
    navigation: Value,
    result: Value,
) -> Value {
    json!({
        "url": url,
        "ok": true,
        "lane": public_lane_json(lane),
        "lane_id": lane.lane_id.as_str(),
        "lifecycle_state": lane.lifecycle_state,
        "identity_mode": lane.identity_mode,
        "browser_epoch": lane.browser_epoch,
        "recommended_concurrency": recommended_concurrency,
        "capacity_or_recovery_hint": lane_next_action(lane),
        "navigation": navigation,
        "result": result,
    })
}

fn crawl_error_result(
    url: String,
    lane: Option<&BrowserLaneSnapshot>,
    identity_mode: BrowserIdentityMode,
    recommended_concurrency: usize,
    error: Value,
) -> Value {
    json!({
        "url": url,
        "ok": false,
        "lane": lane.map(public_lane_json),
        "lane_id": lane.map(|lane| lane.lane_id.as_str()),
        "lifecycle_state": lane.map(|lane| lane.lifecycle_state),
        "identity_mode": lane
            .map(|lane| lane.identity_mode)
            .unwrap_or(identity_mode),
        "browser_epoch": lane.map(|lane| lane.browser_epoch),
        "recommended_concurrency": lane
            .and_then(|lane| lane.queue.as_ref())
            .map(|queue| queue.recommended_concurrency)
            .unwrap_or(recommended_concurrency),
        "capacity_or_recovery_hint": lane
            .map(lane_next_action)
            .unwrap_or_else(|| {
                error
                    .get("next_action")
                    .and_then(Value::as_str)
                    .unwrap_or("Retry this URL in a new bounded crawl batch.")
            }),
        "error": error,
    })
}

fn crawl_lane_unavailable_result(
    url: &str,
    lane: &BrowserLaneSnapshot,
    recommended_concurrency: usize,
) -> Value {
    let (code, message, retryable, next_action) = match lane.lifecycle_state {
        LaneLifecycleState::Queued => (
            "browser_capacity_queued",
            "The crawl Lane was queued and no URL work was dispatched.",
            true,
            "Retry later, lower crawl concurrency, or reuse an already-running Lane.",
        ),
        LaneLifecycleState::Starting => (
            "crawl_lane_starting",
            "The crawl Lane did not become running before this batch returned.",
            true,
            "Retry after the Lane finishes starting.",
        ),
        LaneLifecycleState::Frozen => (
            "crawl_lane_frozen",
            "The crawl Lane is frozen because capacity is pressured.",
            true,
            "Retry later or lower crawl concurrency.",
        ),
        LaneLifecycleState::Stopping => (
            "crawl_lane_stopping",
            "The crawl Lane was stopping before URL work could be dispatched.",
            true,
            "Retry this URL in a new bounded crawl batch.",
        ),
        LaneLifecycleState::Failed => (
            "crawl_lane_failed",
            "The crawl Lane failed before URL work could be dispatched.",
            lane.recoverable,
            "Inspect the Lane recovery fields and retry only when advised.",
        ),
        LaneLifecycleState::Running => (
            "crawl_lane_unavailable",
            "The crawl Lane was unexpectedly unavailable.",
            true,
            "Retry this URL in a new bounded crawl batch.",
        ),
    };
    let mut result = crawl_error_result(
        url.to_owned(),
        Some(lane),
        lane.identity_mode,
        recommended_concurrency,
        json!({
            "code": code,
            "message": message,
            "retryable": retryable,
            "next_action": next_action,
        }),
    );
    result["dispatched"] = Value::Bool(false);
    result
}

fn crawl_worker_terminal_failure(
    url: &str,
    lane: &BrowserLaneSnapshot,
    recommended_concurrency: usize,
    cause: &str,
) -> Value {
    crawl_error_result(
        url.to_owned(),
        Some(lane),
        lane.identity_mode,
        recommended_concurrency,
        json!({
            "code": "crawl_worker_failed",
            "message": "The crawl worker terminated before producing this URL result.",
            "retryable": true,
            "next_action": "Retry this URL in a new bounded crawl batch.",
            "cause": cause,
        }),
    )
}

fn attach_crawl_cleanup(
    result: &mut Value,
    cleanup: &Result<CloseResult, BrowserPlatformError>,
) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    match cleanup {
        Ok(close) => {
            object.insert(
                "cleanup".to_owned(),
                json!({
                    "closed": true,
                    "closed_count": close.closed,
                    "already_closed": close.already_closed,
                }),
            );
        }
        Err(error) => {
            object.insert("ok".to_owned(), Value::Bool(false));
            object.insert(
                "cleanup".to_owned(),
                json!({
                    "closed": false,
                    "error": public_platform_error_json(error),
                }),
            );
        }
    }
}

async fn close_lane_ids_until(
    client: Arc<dyn BrowserLaneClientPort>,
    lane_ids: Vec<BrowserLaneId>,
    deadline: tokio::time::Instant,
) -> HashMap<BrowserLaneId, Result<CloseResult, BrowserPlatformError>> {
    let mut workers = tokio::task::JoinSet::new();
    let mut task_lanes = HashMap::new();
    for lane_id in lane_ids.iter().cloned() {
        let worker_client = Arc::clone(&client);
        let worker_lane_id = lane_id.clone();
        let abort_handle =
            workers.spawn(async move { worker_client.close(&worker_lane_id).await });
        task_lanes.insert(abort_handle.id(), lane_id);
    }
    let mut results = HashMap::new();
    loop {
        let joined = match tokio::time::timeout_at(deadline, workers.join_next_with_id()).await {
            Ok(joined) => joined,
            Err(_) => break,
        };
        let Some(joined) = joined else {
            break;
        };
        match joined {
            Ok((task_id, result)) => {
                if let Some(lane_id) = task_lanes.remove(&task_id) {
                    results.insert(lane_id, result);
                }
            }
            Err(join_error) => {
                if let Some(lane_id) = task_lanes.remove(&join_error.id()) {
                    results.insert(
                        lane_id.clone(),
                        Err(BrowserPlatformError::new(
                            nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                            "The crawl Lane cleanup worker terminated unexpectedly.",
                            true,
                            "Retry closing the Lane.",
                        )
                        .for_lane(lane_id)),
                    );
                }
            }
        }
    }
    workers.abort_all();
    for lane_id in task_lanes.into_values() {
        results.insert(
            lane_id.clone(),
            Err(BrowserPlatformError::new(
                nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                "The crawl batch cleanup deadline expired.",
                true,
                "Retry closing the Lane.",
            )
            .for_lane(lane_id)),
        );
    }
    results
}

async fn cleanup_dropped_crawl_batch(
    client: Arc<dyn BrowserLaneClientPort>,
    lane_ids: Vec<BrowserLaneId>,
    pending_names: HashSet<String>,
) {
    let deadline = tokio::time::Instant::now() + CRAWL_CLEANUP_TIMEOUT;
    let known_ids = lane_ids.iter().cloned().collect::<HashSet<_>>();
    let results = close_lane_ids_until(Arc::clone(&client), lane_ids, deadline).await;
    for (lane_id, result) in &results {
        if let Err(error) = result {
            tracing::warn!(
                lane_id = %lane_id,
                code = ?error.code,
                "managed crawl RAII Lane cleanup failed"
            );
        }
    }
    if pending_names.is_empty() || tokio::time::Instant::now() >= deadline {
        return;
    }
    let lanes = match tokio::time::timeout_at(deadline, client.list()).await {
        Ok(Ok(lanes)) => lanes,
        Ok(Err(error)) => {
            tracing::warn!(
                code = ?error.code,
                "managed crawl RAII cleanup could not resolve a cancelled open"
            );
            return;
        }
        Err(_) => {
            tracing::warn!("managed crawl RAII cleanup deadline expired resolving an open");
            return;
        }
    };
    let unresolved_ids = lanes
        .into_iter()
        .filter(|lane| {
            pending_names.contains(&lane.lane_key.lane_name)
                && !known_ids.contains(&lane.lane_id)
        })
        .map(|lane| lane.lane_id)
        .collect::<Vec<_>>();
    let _ = close_lane_ids_until(client, unresolved_ids, deadline).await;
}

fn managed_lane_id(input: &Value) -> Result<Option<BrowserLaneId>, BrowserPlatformError> {
    match input.get("lane_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => BrowserLaneId::parse(value.to_owned()).map(Some),
        Some(_) => Err(BrowserPlatformError::new(
            nomifun_browser_platform::BrowserErrorCode::LaneNotFound,
            "`lane_id` must be a Lane handle string returned by the Browser Platform.",
            false,
            "Use a lane_id returned by browser_open, browser_fork, or browser_list.",
        )),
    }
}

fn parse_crawl_request(
    input: &Value,
    workspace_dir: Option<&std::path::Path>,
) -> Result<ManagedCrawlRequest, String> {
    if let Some(field) = first_model_identity_field(input) {
        return Err(format!(
            "Browser input field `{field}` selects trusted host identity policy and is not accepted. \
             browser_crawl_many uses Anonymous identity until a trusted host planner selects otherwise."
        ));
    }
    let urls = crawl_urls(input)?;
    let requested_concurrency = crawl_concurrency(input, urls.len())?;
    Ok(ManagedCrawlRequest {
        urls,
        requested_concurrency,
        auto_concurrency: crawl_concurrency_is_auto(input),
        identity_mode: BrowserIdentityMode::Anonymous,
        schema: input.get("schema").cloned(),
        workspace_hint: workspace_dir.map(|path| path.to_string_lossy().into_owned()),
    })
}

fn crawl_urls(input: &Value) -> Result<Vec<String>, String> {
    let Some(raw_urls) = input.get("urls") else {
        return Err(
            "Missing required `urls` array for browser_crawl_many (one or more HTTP(S) URLs)."
                .to_owned(),
        );
    };
    let Some(values) = raw_urls.as_array() else {
        return Err("`urls` must be an array of HTTP(S) URL strings.".to_owned());
    };
    if values.is_empty() {
        return Err("`urls` must contain at least one URL.".to_owned());
    }
    if values.len() > MAX_CRAWL_URLS {
        return Err(format!(
            "`urls` is bounded to {MAX_CRAWL_URLS} entries per browser_crawl_many call."
        ));
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let url = value
                .as_str()
                .map(str::trim)
                .ok_or_else(|| format!("`urls[{index}]` must be a string."))?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(format!("`urls[{index}]` must be an HTTP(S) URL."));
            }
            Ok(url.to_owned())
        })
        .collect()
}

fn crawl_concurrency(input: &Value, url_count: usize) -> Result<usize, String> {
    let requested = match input.get("concurrency") {
        None => 4,
        Some(Value::String(value)) if value == "auto" => 4,
        Some(Value::Number(number)) => {
            let value = number.as_u64().ok_or_else(|| {
                "`concurrency` must be \"auto\" or an integer from 1 through 8.".to_owned()
            })?;
            if !(1..=MAX_CRAWL_CONCURRENCY as u64).contains(&value) {
                return Err(format!(
                    "`concurrency` must be between 1 and {MAX_CRAWL_CONCURRENCY}; received {value}."
                ));
            }
            value as usize
        }
        Some(Value::String(value)) => {
            return Err(format!(
                "Invalid `concurrency` {value:?}; expected \"auto\" or an integer from 1 through 8."
            ));
        }
        Some(_) => {
            return Err(
                "`concurrency` must be \"auto\" or an integer from 1 through 8.".to_owned(),
            );
        }
    };
    Ok(requested.min(url_count))
}

fn crawl_concurrency_is_auto(input: &Value) -> bool {
    matches!(input.get("concurrency"), Some(Value::String(value)) if value == "auto")
        || input.get("concurrency").is_none()
}

fn first_model_identity_field(input: &Value) -> Option<&'static str> {
    let object = input.as_object()?;
    MODEL_IDENTITY_INPUT_FIELDS
        .iter()
        .copied()
        .find(|field| object.contains_key(*field))
}

fn first_trusted_owner_field(input: &Value) -> Option<&'static str> {
    let object = input.as_object()?;
    TRUSTED_OWNER_INPUT_FIELDS
        .iter()
        .copied()
        .find(|field| object.contains_key(*field))
}

fn sanitize_operation_input(input: &Value) -> Value {
    let mut sanitized = input.as_object().cloned().unwrap_or_default();
    sanitized.remove("lane_id");
    sanitized.remove("lane_name");
    sanitized.remove("expected_browser_epoch");
    sanitized.remove(OUT_OF_BAND_CONFIRMED_KEY);
    for field in TRUSTED_OWNER_INPUT_FIELDS {
        sanitized.remove(*field);
    }
    Value::Object(sanitized)
}

fn is_existing_browser_action(action: &str) -> bool {
    matches!(
        action,
        "navigate"
            | "observe"
            | "screenshot"
            | "capabilities"
            | "get_page_text"
            | "search_page"
            | "find_elements"
            | "get_dropdown_options"
            | "cursor"
            | "tabs"
            | "wait"
            | "wait_for"
            | "get_console_logs"
            | "get_page_errors"
            | "get_network_log"
            | "click"
            | "hover"
            | "type"
            | "set_value"
            | "select_option"
            | "press_key"
            | "scroll"
            | "scroll_to_text"
            | "upload_file"
            | "download"
            | "save_as_pdf"
            | "extract"
            | "switch_frame"
            | "switch_tab"
            | "close_tab"
            | "open_link_new_tab"
            | "back"
            | "forward"
            | "reload"
            | "evaluate"
    )
}

fn operation_kind(action: &str) -> BrowserOperationKind {
    match action {
        "navigate" | "back" | "forward" | "reload" => BrowserOperationKind::Navigate,
        "observe"
        | "get_page_text"
        | "search_page"
        | "find_elements"
        | "get_dropdown_options"
        | "cursor" => BrowserOperationKind::Observe,
        "screenshot" => BrowserOperationKind::Screenshot,
        "tabs" | "switch_tab" | "close_tab" | "open_link_new_tab" => {
            BrowserOperationKind::Tabs
        }
        "download" | "save_as_pdf" => BrowserOperationKind::Download,
        "get_console_logs" | "get_page_errors" | "get_network_log" | "evaluate" => {
            BrowserOperationKind::Debug
        }
        "capabilities" => BrowserOperationKind::Manage,
        _ => BrowserOperationKind::Act,
    }
}

fn action_may_modify_identity(action: &str, input: &Value) -> bool {
    match action {
        // Pure navigation is a supported authenticated-replica read path.
        // An explicitly state-changing request shape is still refused.
        "navigate" | "back" | "forward" | "reload" | "open_link_new_tab" => {
            input_declares_stateful_request(input)
        }
        // These actions can submit forms, run page code, mutate storage, or
        // change durable account state.
        "click"
        | "type"
        | "set_value"
        | "select_option"
        | "press_key"
        | "upload_file"
        | "evaluate" => true,
        // Every currently supported observation, local cursor, wait, frame,
        // tab-selection/close, extraction, download, and presentation action
        // is an additional-hint false. The Hub remains the trusted classifier.
        "observe"
        | "screenshot"
        | "capabilities"
        | "get_page_text"
        | "search_page"
        | "find_elements"
        | "get_dropdown_options"
        | "cursor"
        | "tabs"
        | "wait"
        | "wait_for"
        | "get_console_logs"
        | "get_page_errors"
        | "get_network_log"
        | "hover"
        | "scroll"
        | "scroll_to_text"
        | "download"
        | "save_as_pdf"
        | "extract"
        | "switch_frame"
        | "switch_tab"
        | "close_tab" => false,
        // Fail closed if a future supported action reaches this classifier
        // without an explicit security review.
        _ => true,
    }
}

fn input_declares_stateful_request(input: &Value) -> bool {
    input
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| {
            !method.eq_ignore_ascii_case("get") && !method.eq_ignore_ascii_case("head")
        })
        || input
            .get("submits_form")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Safe owner-visible Lane DTO shared by all tool transports.
pub fn public_lane_json(lane: &BrowserLaneSnapshot) -> Value {
    let tabs = lane
        .tabs
        .iter()
        .map(|tab| {
            json!({
                "tab_id": tab.tab_id,
                "title": tab.title,
                "url": tab.url,
                "active": tab.active,
                "crashed": tab.crashed,
            })
        })
        .collect::<Vec<_>>();
    let queue = lane.queue.as_ref().map(|queue| {
        json!({
            "position": queue.position,
            "recommended_concurrency": queue.recommended_concurrency,
            "owner_active": queue.owner_active,
            "owner_queued": queue.owner_queued,
            "global_active": queue.global_active,
            "global_queued": queue.global_queued,
            "retry_delay_ms": queue.retry_delay_ms,
            "reason_code": queue.reason_code,
        })
    });
    json!({
        "lane_id": lane.lane_id.as_str(),
        "lane_name": lane.lane_key.lane_name,
        "identity_mode": lane.identity_mode,
        "identity_generation": lane.identity_generation,
        "lifecycle_state": lane.lifecycle_state,
        "control_state": lane.control_state,
        "browser_epoch": lane.browser_epoch,
        "tabs": tabs,
        "active_tab_id": lane.active_tab_id,
        "ref_generation": lane.ref_generation,
        "queue": queue,
        "recommended_concurrency": lane
            .queue
            .as_ref()
            .map(|queue| queue.recommended_concurrency),
        "recoverable": lane.recoverable,
        "error_code": lane.error_code,
        "error_message": lane.error_message,
    })
}

fn lane_next_action(lane: &BrowserLaneSnapshot) -> &'static str {
    match lane.lifecycle_state {
        LaneLifecycleState::Queued => {
            "Wait for the Lane, reuse an existing Lane, or lower concurrency."
        }
        LaneLifecycleState::Starting => "Wait for the Lane to finish starting, then retry.",
        LaneLifecycleState::Running => "Use the returned lane_id for browser operations.",
        LaneLifecycleState::Frozen => "Reuse an active Lane or wait for capacity to recover.",
        LaneLifecycleState::Stopping => "Open a replacement Lane only if more work is required.",
        LaneLifecycleState::Failed => {
            "Inspect error_code and recoverable; open a replacement Lane when advised."
        }
    }
}

fn operation_result(
    action: &str,
    lane_id: &BrowserLaneId,
    result: BrowserOperationResult,
    lane: Option<&BrowserLaneSnapshot>,
) -> ToolResult {
    let mut output = result.output;
    let mut images = Vec::new();
    if action == "screenshot"
        && let Some(object) = output.as_object_mut()
        && let (Some(media_type), Some(data)) = (
            object.get("media_type").and_then(Value::as_str),
            object.get("data").and_then(Value::as_str),
        )
    {
        images.push(ToolImage {
            media_type: media_type.to_owned(),
            data: data.to_owned(),
        });
        object.remove("data");
        object.insert("image_attached".to_owned(), Value::Bool(true));
    }
    let mut envelope = json!({
        "ok": true,
        "action": action,
        "lane_id": lane_id.as_str(),
        "lane": lane.map(public_lane_json),
        "output": output.clone(),
        "ref_generation": result.ref_generation,
    });
    // Keep the legacy transport shape usable while adding the cross-entry
    // Lane metadata: existing consumers may still read `/result/text`,
    // `/result/yaml`, `/result/final_url`, etc. New consumers can use `output`.
    if let Some(envelope) = envelope.as_object_mut() {
        match &output {
            Value::Object(fields) => {
                for (key, value) in fields {
                    envelope.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
            Value::String(text) => {
                envelope
                    .entry("text".to_owned())
                    .or_insert_with(|| Value::String(text.clone()));
            }
            _ => {}
        }
        if action == "screenshot" {
            envelope.insert("captured".to_owned(), Value::Bool(true));
        }
    }
    ToolResult::text(pretty_json(&envelope))
    .with_images(images)
}

fn public_platform_error_json(error: &BrowserPlatformError) -> Value {
    json!({
        "code": error.code,
        "message": error.message,
        "retryable": error.retryable,
        "next_action": error.next_action,
        "lane_id": error.lane_id.as_ref().map(BrowserLaneId::as_str),
        "metadata": error.metadata,
    })
}

fn platform_error_result(context: &str, error: BrowserPlatformError) -> ToolResult {
    ToolResult::error(pretty_json(&json!({
        "ok": false,
        "context": context,
        "error": public_platform_error_json(&error),
    })))
}

pub(crate) fn invalid_browser_request(message: impl Into<String>) -> ToolResult {
    ToolResult::error(pretty_json(&json!({
        "ok": false,
        "error": {
            "code": INVALID_CRAWL_REQUEST_CODE,
            "message": message.into(),
            "retryable": false,
            "next_action": "Correct the Browser tool arguments and retry.",
        }
    })))
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_tools::Tool;
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    #[derive(Default)]
    struct FakeLaneClient {
        opens: Mutex<Vec<(Option<String>, BrowserIdentityMode, Option<String>)>>,
        operations: Mutex<Vec<(BrowserLaneId, BrowserOperation)>>,
        closes: Mutex<Vec<BrowserLaneId>>,
        lanes: Mutex<Vec<BrowserLaneSnapshot>>,
        sequence: AtomicU64,
        open_lifecycle: Mutex<Option<LaneLifecycleState>>,
        panic_url: Mutex<Option<String>>,
        block_open_after_insert: AtomicBool,
        block_execute: AtomicBool,
        open_started: tokio::sync::Notify,
        open_release: tokio::sync::Notify,
        execute_started: tokio::sync::Notify,
        execute_release: tokio::sync::Notify,
        close_called: tokio::sync::Notify,
    }

    impl FakeLaneClient {
        fn snapshot(
            &self,
            lane_name: &str,
            identity_mode: BrowserIdentityMode,
        ) -> BrowserLaneSnapshot {
            let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
            BrowserLaneSnapshot {
                lane_id: BrowserLaneId(format!("managed-lane-{sequence}")),
                lane_key: nomifun_browser_platform::LaneKey {
                    runtime_instance_id: "managed-runtime".to_owned(),
                    lane_name: lane_name.to_owned(),
                },
                caller: nomifun_browser_platform::CallerIdentity {
                    user_id: "managed-user".to_owned(),
                    conversation_id: Some("managed-conversation".to_owned()),
                    runtime_instance_id: "managed-runtime".to_owned(),
                    agent_id: Some("managed-agent".to_owned()),
                    companion_id: None,
                    execution_id: None,
                    step_id: None,
                    attempt_id: None,
                    remote_connection_id: None,
                    surface: nomifun_browser_platform::BrowserSurface::Native,
                    owner_lease_id: nomifun_browser_platform::OwnerLeaseId(
                        "managed-owner-lease".to_owned(),
                    ),
                    capability_expires_at_ms: u64::MAX,
                    allowed_operations: BTreeSet::from([
                        BrowserOperationKind::Manage,
                        BrowserOperationKind::Navigate,
                        BrowserOperationKind::Observe,
                        BrowserOperationKind::Act,
                        BrowserOperationKind::Screenshot,
                        BrowserOperationKind::Tabs,
                        BrowserOperationKind::Download,
                        BrowserOperationKind::Debug,
                        BrowserOperationKind::Crawl,
                    ]),
                },
                identity_mode,
                identity_generation: 1,
                lifecycle_state: LaneLifecycleState::Running,
                control_state: nomifun_browser_platform::LaneControlState::Agent,
                browser_epoch: 1,
                tabs: Vec::new(),
                active_tab_id: None,
                active_frame_id: None,
                ref_generation: 0,
                queue: None,
                resource_estimate_bytes: 0,
                active_operation_count: 0,
                last_active_at_ms: 0,
                created_at_ms: sequence,
                viewer_state: nomifun_browser_platform::ViewerState::Idle,
                error_code: None,
                error_message: None,
                recoverable: false,
            }
        }
    }

    #[async_trait]
    impl BrowserLaneClientPort for FakeLaneClient {
        async fn open(
            &self,
            lane_name: Option<&str>,
            identity_mode: BrowserIdentityMode,
            workspace_hint: Option<String>,
        ) -> Result<OpenLaneOutcome, BrowserPlatformError> {
            self.opens.lock().unwrap().push((
                lane_name.map(str::to_owned),
                identity_mode,
                workspace_hint,
            ));
            let lane_name = lane_name.unwrap_or("default");
            if let Some(existing) = self
                .lanes
                .lock()
                .unwrap()
                .iter()
                .find(|lane| lane.lane_key.lane_name == lane_name)
                .cloned()
            {
                return if existing.lifecycle_state == LaneLifecycleState::Queued {
                    Ok(OpenLaneOutcome::Queued { lane: existing })
                } else {
                    Ok(OpenLaneOutcome::Running { lane: existing })
                };
            }
            let mut lane = self.snapshot(lane_name, identity_mode);
            if let Some(lifecycle) = self.open_lifecycle.lock().unwrap().clone() {
                lane.lifecycle_state = lifecycle;
            }
            self.lanes.lock().unwrap().push(lane.clone());
            if self.block_open_after_insert.load(Ordering::Acquire) {
                self.open_started.notify_one();
                self.open_release.notified().await;
            }
            if lane.lifecycle_state == LaneLifecycleState::Queued {
                Ok(OpenLaneOutcome::Queued { lane })
            } else {
                Ok(OpenLaneOutcome::Running { lane })
            }
        }

        async fn execute(
            &self,
            lane_id: &BrowserLaneId,
            operation: BrowserOperation,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            if !self
                .lanes
                .lock()
                .unwrap()
                .iter()
                .any(|lane| &lane.lane_id == lane_id)
            {
                return Err(BrowserPlatformError::lane_not_found(lane_id.clone()));
            }
            let action = operation.action.clone();
            let operation_url = operation
                .input
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_owned);
            self.operations
                .lock()
                .unwrap()
                .push((lane_id.clone(), operation));
            if self.block_execute.load(Ordering::Acquire) {
                self.execute_started.notify_one();
                self.execute_release.notified().await;
            }
            let should_panic = action == "navigate"
                && self
                    .panic_url
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|panic_url| operation_url.as_deref() == Some(panic_url.as_str()));
            assert!(
                !should_panic,
                "intentional managed crawl worker panic for regression coverage"
            );
            Ok(BrowserOperationResult {
                output: match action.as_str() {
                    "navigate" => json!({
                        "final_url": operation_url,
                        "http_status": 200,
                        "redirected": false,
                        "load_state": "complete",
                    }),
                    "get_page_text" => json!({ "message": "managed fixture text" }),
                    _ => json!({ "message": format!("{action} ok") }),
                },
                ..BrowserOperationResult::default()
            })
        }

        async fn list(&self) -> Result<Vec<BrowserLaneSnapshot>, BrowserPlatformError> {
            Ok(self.lanes.lock().unwrap().clone())
        }

        async fn status(
            &self,
            lane_id: &BrowserLaneId,
        ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
            self.lanes
                .lock()
                .unwrap()
                .iter()
                .find(|lane| &lane.lane_id == lane_id)
                .cloned()
                .ok_or_else(|| BrowserPlatformError::lane_not_found(lane_id.clone()))
        }

        async fn close(
            &self,
            lane_id: &BrowserLaneId,
        ) -> Result<CloseResult, BrowserPlatformError> {
            self.closes.lock().unwrap().push(lane_id.clone());
            let mut lanes = self.lanes.lock().unwrap();
            let before = lanes.len();
            lanes.retain(|lane| &lane.lane_id != lane_id);
            let closed = usize::from(lanes.len() != before);
            self.close_called.notify_one();
            Ok(CloseResult {
                closed,
                already_closed: closed == 0,
            })
        }

        async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError> {
            let mut lanes = self.lanes.lock().unwrap();
            let closed = lanes.len();
            lanes.clear();
            Ok(CloseResult {
                closed,
                already_closed: closed == 0,
            })
        }
    }

    fn facade(client: Arc<FakeLaneClient>) -> ManagedBrowserFacade {
        ManagedBrowserFacade {
            client,
            workspace_dir: None,
        }
    }

    #[test]
    fn result_envelope_preserves_structured_success_error_and_images() {
        let success = managed_result_envelope(
            ToolResult::text(r#"{"ok":true}"#).with_images(vec![ToolImage {
                media_type: "image/png".to_owned(),
                data: "QUJD".to_owned(),
            }]),
        );
        assert_eq!(success.pointer("/result/ok"), Some(&Value::Bool(true)));
        assert_eq!(
            success.pointer("/_mcp_images/0/mime_type").and_then(Value::as_str),
            Some("image/png")
        );

        let error = managed_result_envelope(ToolResult::error(r#"{"ok":false}"#));
        assert_eq!(error.pointer("/error/ok"), Some(&Value::Bool(false)));
    }

    #[test]
    fn existing_action_keeps_legacy_output_fields_with_lane_metadata() {
        let lane_id = BrowserLaneId::parse("lane-compatible").unwrap();
        let result = operation_result(
            "navigate",
            &lane_id,
            BrowserOperationResult {
                output: json!({
                    "text": "Navigated",
                    "final_url": "https://example.test/final",
                }),
                ..Default::default()
            },
            None,
        );
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["text"], "Navigated");
        assert_eq!(value["final_url"], "https://example.test/final");
        assert_eq!(value["output"]["text"], "Navigated");
        assert_eq!(value["lane_id"], "lane-compatible");
    }

    #[test]
    fn owner_fields_are_rejected_but_lane_selection_is_not_owner_construction() {
        assert_eq!(
            first_trusted_owner_field(&json!({"runtime_instance_id": "forged"})),
            Some("runtime_instance_id")
        );
        assert_eq!(
            first_trusted_owner_field(&json!({"lane_id": "owner-scoped-handle"})),
            None
        );
    }

    #[test]
    fn model_identity_fields_are_classified_as_untrusted_input() {
        for field in MODEL_IDENTITY_INPUT_FIELDS {
            let mut input = serde_json::Map::new();
            input.insert((*field).to_owned(), json!("model-selected"));
            assert_eq!(
                first_model_identity_field(&Value::Object(input)),
                Some(*field)
            );
        }
    }

    #[test]
    fn identity_modification_hint_is_closed_without_blocking_replica_navigation() {
        for action in [
            "evaluate",
            "click",
            "type",
            "set_value",
            "select_option",
            "press_key",
            "upload_file",
        ] {
            assert!(
                action_may_modify_identity(action, &json!({})),
                "{action} must carry the conservative replica write hint"
            );
        }

        for action in [
            "navigate",
            "back",
            "forward",
            "reload",
            "open_link_new_tab",
            "close_tab",
            "observe",
            "screenshot",
            "capabilities",
            "get_page_text",
            "search_page",
            "find_elements",
            "get_dropdown_options",
            "cursor",
            "tabs",
            "switch_tab",
            "get_console_logs",
            "get_page_errors",
            "get_network_log",
            "download",
            "save_as_pdf",
            "hover",
            "scroll",
            "scroll_to_text",
            "wait",
            "wait_for",
            "extract",
            "switch_frame",
        ] {
            assert!(
                !action_may_modify_identity(action, &json!({})),
                "{action} should remain usable as a non-mutating hint"
            );
        }
        assert!(action_may_modify_identity(
            "navigate",
            &json!({"method": "POST"})
        ));
        assert!(action_may_modify_identity("future_action", &json!({})));
    }

    #[tokio::test]
    async fn browser_open_and_fork_always_use_primary_identity() {
        let client = Arc::new(FakeLaneClient::default());
        let facade = facade(Arc::clone(&client));
        let native_client = Arc::new(crate::tool::tests::FakeLaneClient::default());
        let native_tool =
            crate::tool::tests::managed_tool_for_contract(Arc::clone(&native_client));

        let opened = facade
            .execute("browser_open", &json!({"lane_name": "interactive"}))
            .await;
        assert!(!opened.is_error, "{}", opened.content);
        let opened_value: Value = serde_json::from_str(&opened.content).unwrap();
        let native_opened = native_tool
            .execute(json!({
                "action": "browser_open",
                "lane_name": "native-interactive",
            }))
            .await;
        assert!(!native_opened.is_error, "{}", native_opened.content);
        let native_opened_value: Value = serde_json::from_str(&native_opened.content).unwrap();
        assert_eq!(opened_value["lane"]["identity_mode"], "primary");
        assert_eq!(
            native_opened_value["lane"]["identity_mode"],
            opened_value["lane"]["identity_mode"]
        );

        let forked = facade
            .execute(
                "browser_fork",
                &json!({"lane_name": "interactive-fork"}),
            )
            .await;
        assert!(!forked.is_error, "{}", forked.content);
        let forked_value: Value = serde_json::from_str(&forked.content).unwrap();
        let native_forked = native_tool
            .execute(json!({
                "action": "browser_fork",
                "lane_name": "native-interactive-fork",
            }))
            .await;
        assert!(!native_forked.is_error, "{}", native_forked.content);
        let native_forked_value: Value = serde_json::from_str(&native_forked.content).unwrap();
        assert_eq!(forked_value["lane"]["identity_mode"], "primary");
        assert_eq!(
            native_forked_value["lane"]["identity_mode"],
            forked_value["lane"]["identity_mode"]
        );

        let opens = client.opens.lock().unwrap();
        assert_eq!(opens.len(), 2);
        assert!(
            opens
                .iter()
                .all(|(_, identity_mode, _)| *identity_mode == BrowserIdentityMode::Primary),
            "model-facing open/fork must never select a non-Primary identity: {opens:?}"
        );
    }

    #[tokio::test]
    async fn model_identity_fields_fail_closed_before_open_or_crawl() {
        for action in ["browser_open", "browser_fork", "browser_crawl_many"] {
            for field in MODEL_IDENTITY_INPUT_FIELDS {
                let client = Arc::new(FakeLaneClient::default());
                let facade = facade(Arc::clone(&client));
                let mut object = serde_json::Map::new();
                if action == "browser_crawl_many" {
                    object.insert(
                        "urls".to_owned(),
                        json!(["https://example.test/identity-policy"]),
                    );
                }
                object.insert((*field).to_owned(), json!("model-selected"));

                let result = facade.execute(action, &Value::Object(object)).await;
                assert!(result.is_error, "{action} with {field} unexpectedly succeeded");
                let value: Value = serde_json::from_str(&result.content).unwrap();
                assert_eq!(
                    value["error"]["code"],
                    "invalid_browser_request",
                    "{action} with {field}: {value}"
                );
                assert!(
                    client.opens.lock().unwrap().is_empty(),
                    "{action} with {field} must not open a Lane"
                );
                assert!(
                    client.operations.lock().unwrap().is_empty(),
                    "{action} with {field} must not execute crawl work"
                );
            }
        }
    }

    #[test]
    fn crawl_inputs_are_bounded_and_orderable() {
        assert_eq!(crawl_concurrency(&json!({}), 70).unwrap(), 4);
        assert_eq!(
            crawl_concurrency(&json!({"concurrency": "auto"}), 2).unwrap(),
            2
        );
        assert_eq!(crawl_concurrency(&json!({"concurrency": 1}), 70).unwrap(), 1);
        assert_eq!(
            crawl_concurrency(&json!({"concurrency": MAX_CRAWL_CONCURRENCY}), 70).unwrap(),
            MAX_CRAWL_CONCURRENCY
        );
        for invalid in [
            json!({"concurrency": 0}),
            json!({"concurrency": -1}),
            json!({"concurrency": MAX_CRAWL_CONCURRENCY + 1}),
            json!({"concurrency": u64::MAX}),
            json!({"concurrency": 1.5}),
            json!({"concurrency": 4.0}),
            json!({"concurrency": "4"}),
            json!({"concurrency": "AUTO"}),
            json!({"concurrency": false}),
            json!({"concurrency": null}),
            json!({"concurrency": {}}),
            json!({"concurrency": []}),
        ] {
            assert!(crawl_concurrency(&invalid, 70).is_err(), "{invalid}");
        }
        assert!(crawl_urls(&json!({"urls": ["file:///etc/passwd"]})).is_err());
        assert!(
            crawl_urls(&json!({"urls": (0..=MAX_CRAWL_URLS)
                .map(|index| format!("https://example.test/{index}"))
                .collect::<Vec<_>>() }))
            .is_err()
        );
    }

    fn assert_crawl_item_contract(item: &Value) {
        for field in [
            "lane_id",
            "lifecycle_state",
            "identity_mode",
            "browser_epoch",
            "recommended_concurrency",
            "capacity_or_recovery_hint",
        ] {
            assert!(
                item.get(field).is_some(),
                "crawl item is missing {field}: {item}"
            );
        }
    }

    #[tokio::test]
    async fn native_and_managed_crawl_many_share_golden_payload_shape_and_errors() {
        let input = json!({
            "action": "browser_crawl_many",
            "urls": [
                " https://example.test/first ",
                "https://example.test/second",
            ],
            "concurrency": "auto",
        });

        let managed_client = Arc::new(FakeLaneClient::default());
        let managed = facade(Arc::clone(&managed_client))
            .execute("browser_crawl_many", &input)
            .await;
        assert!(!managed.is_error, "{}", managed.content);
        let managed_value: Value = serde_json::from_str(&managed.content).unwrap();
        assert_eq!(managed_value["requested"], 2);
        assert_eq!(managed_value["identity_mode"], "anonymous");
        assert!(
            managed_client
                .opens
                .lock()
                .unwrap()
                .iter()
                .all(|(_, identity_mode, _)| *identity_mode == BrowserIdentityMode::Anonymous),
            "model-facing crawl_many must default to Anonymous identity"
        );
        let managed_results = managed_value["results"].as_array().unwrap();
        assert_eq!(managed_results.len(), 2);
        assert_eq!(managed_results[0]["url"], "https://example.test/first");
        assert_eq!(managed_results[1]["url"], "https://example.test/second");
        for item in managed_results {
            assert_crawl_item_contract(item);
        }

        let native_client = Arc::new(crate::tool::tests::FakeLaneClient::default());
        let native_tool = crate::tool::tests::managed_tool_for_contract(Arc::clone(&native_client));
        let native = native_tool.execute(input).await;
        assert!(!native.is_error, "{}", native.content);
        let native_value: Value = serde_json::from_str(&native.content).unwrap();
        assert_eq!(native_value["requested"], managed_value["requested"]);
        assert_eq!(native_value["identity_mode"], managed_value["identity_mode"]);
        let native_results = native_value["results"].as_array().unwrap();
        assert_eq!(native_results.len(), managed_results.len());
        for (native_item, managed_item) in native_results.iter().zip(managed_results) {
            assert_eq!(native_item["url"], managed_item["url"]);
            assert_crawl_item_contract(native_item);
            assert_eq!(
                native_item
                    .as_object()
                    .unwrap()
                    .keys()
                    .collect::<HashSet<_>>(),
                managed_item
                    .as_object()
                    .unwrap()
                    .keys()
                    .collect::<HashSet<_>>(),
                "Native and Managed crawl item key sets must remain identical"
            );
        }

        for invalid_input in [
            json!({"action": "browser_crawl_many"}),
            json!({"action": "browser_crawl_many", "urls": ["file:///tmp/nope"]}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "concurrency": 0}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "concurrency": 9}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "concurrency": {}}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "concurrency": false}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "identity_mode": "bogus"}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "identity_mode": 7}),
            json!({"action": "browser_crawl_many", "urls": ["https://example.test"], "authenticated": "yes"}),
        ] {
            let managed_error = facade(Arc::new(FakeLaneClient::default()))
                .execute("browser_crawl_many", &invalid_input)
                .await;
            assert!(managed_error.is_error);
            let managed_error_value: Value =
                serde_json::from_str(&managed_error.content).unwrap();
            assert_eq!(managed_error_value["error"]["code"], "invalid_browser_request");

            let native_error = crate::tool::tests::managed_tool_for_contract(Arc::new(
                crate::tool::tests::FakeLaneClient::default(),
            ))
            .execute(invalid_input)
            .await;
            assert!(native_error.is_error);
            let native_error_value: Value = serde_json::from_str(&native_error.content).unwrap();
            assert_eq!(native_error_value["error"]["code"], "invalid_browser_request");
        }
    }

    #[test]
    fn crawl_cleanup_failure_is_explicit_and_cannot_leave_result_successful() {
        let mut result = json!({
            "url": "https://example.test/cleanup",
            "ok": true,
        });
        let cleanup = Err(BrowserPlatformError::new(
            nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
            "fixture close failure",
            true,
            "Retry cleanup.",
        ));
        attach_crawl_cleanup(&mut result, &cleanup);

        assert_eq!(result["ok"], false);
        assert_eq!(result["cleanup"]["closed"], false);
        assert_eq!(
            result["cleanup"]["error"]["code"],
            "browser_unavailable"
        );
    }

    #[tokio::test]
    async fn crawl_many_reuses_bounded_lanes_preserves_order_and_cleans_up() {
        let client = Arc::new(FakeLaneClient::default());
        let facade = facade(Arc::clone(&client));
        let result = facade
            .execute(
                "browser_crawl_many",
                &json!({
                    "urls": [
                        "https://example.test/0",
                        "https://example.test/1",
                        "https://example.test/2",
                        "https://example.test/3",
                        "https://example.test/4"
                    ],
                    "concurrency": 2,
                }),
            )
            .await;

        assert!(!result.is_error, "{}", result.content);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), 5);
        for (index, item) in results.iter().enumerate() {
            assert_eq!(
                item["url"],
                Value::String(format!("https://example.test/{index}"))
            );
            assert_eq!(item["ok"], true);
            assert_eq!(item["cleanup"]["closed"], true);
        }
        assert_eq!(client.opens.lock().unwrap().len(), 2);
        assert_eq!(client.operations.lock().unwrap().len(), 10);
        assert_eq!(client.closes.lock().unwrap().len(), 2);
        assert!(client.lanes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn crawl_many_queued_returns_one_failure_per_url_and_cleans_queue_lane() {
        let client = Arc::new(FakeLaneClient::default());
        *client.open_lifecycle.lock().unwrap() = Some(LaneLifecycleState::Queued);
        let facade = facade(Arc::clone(&client));
        let result = facade
            .execute(
                "browser_crawl_many",
                &json!({
                    "urls": [
                        "https://example.test/queued-0",
                        "https://example.test/queued-1"
                    ],
                    "concurrency": 1,
                }),
            )
            .await;

        assert!(!result.is_error);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["ok"], false);
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        for (index, item) in results.iter().enumerate() {
            assert_eq!(
                item["url"],
                Value::String(format!("https://example.test/queued-{index}"))
            );
            assert_eq!(item["ok"], false);
            assert_eq!(item["dispatched"], false);
            assert_eq!(item["error"]["code"], "browser_capacity_queued");
            assert_eq!(item["lifecycle_state"], "queued");
            assert_eq!(item["cleanup"]["closed"], true);
        }
        assert!(client.operations.lock().unwrap().is_empty());
        assert_eq!(client.closes.lock().unwrap().len(), 1);
        assert!(client.lanes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn crawl_many_parent_cancellation_aborts_worker_and_raii_closes_lane() {
        let client = Arc::new(FakeLaneClient::default());
        client.block_execute.store(true, Ordering::Release);
        let facade = facade(Arc::clone(&client));
        let task = tokio::spawn(async move {
            facade
                .execute(
                    "browser_crawl_many",
                    &json!({
                        "urls": [
                            "https://example.test/cancel-0",
                            "https://example.test/cancel-1"
                        ],
                        "concurrency": 1,
                    }),
                )
                .await
        });

        tokio::time::timeout(
            Duration::from_secs(2),
            client.execute_started.notified(),
        )
        .await
        .expect("managed crawl worker should start");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), client.close_called.notified())
            .await
            .expect("RAII cleanup should close the managed crawl Lane");
        assert!(client.lanes.lock().unwrap().is_empty());
        let operation_count = client.operations.lock().unwrap().len();
        client.execute_release.notify_waiters();
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(
            client.operations.lock().unwrap().len(),
            operation_count,
            "the aborted worker must not resume as detached work"
        );
    }

    #[tokio::test]
    async fn crawl_many_cancellation_during_open_closes_lane_by_name() {
        let client = Arc::new(FakeLaneClient::default());
        client
            .block_open_after_insert
            .store(true, Ordering::Release);
        let facade = facade(Arc::clone(&client));
        let task = tokio::spawn(async move {
            facade
                .execute(
                    "browser_crawl_many",
                    &json!({
                        "urls": ["https://example.test/cancel-open"],
                        "concurrency": 1,
                    }),
                )
                .await
        });

        tokio::time::timeout(Duration::from_secs(2), client.open_started.notified())
            .await
            .expect("fake client should register a managed Lane before blocking");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), client.close_called.notified())
            .await
            .expect("RAII cleanup should resolve the deterministic Lane name");
        assert!(client.lanes.lock().unwrap().is_empty());
        assert_eq!(client.closes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn crawl_many_worker_panic_fills_each_assigned_url_in_input_order() {
        let client = Arc::new(FakeLaneClient::default());
        *client.panic_url.lock().unwrap() = Some("https://example.test/2".to_owned());
        let facade = facade(Arc::clone(&client));
        let result = facade
            .execute(
                "browser_crawl_many",
                &json!({
                    "urls": [
                        "https://example.test/0",
                        "https://example.test/1",
                        "https://example.test/2",
                        "https://example.test/3",
                        "https://example.test/4"
                    ],
                    "concurrency": 2,
                }),
            )
            .await;

        assert!(!result.is_error);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["ok"], false);
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), 5);
        for (index, item) in results.iter().enumerate() {
            assert_eq!(
                item["url"],
                Value::String(format!("https://example.test/{index}"))
            );
        }
        for index in [0, 2, 4] {
            assert_eq!(results[index]["ok"], false);
            assert_eq!(results[index]["error"]["code"], "crawl_worker_failed");
            assert_eq!(results[index]["error"]["cause"], "worker_panicked");
            assert_eq!(results[index]["cleanup"]["closed"], true);
        }
        for index in [1, 3] {
            assert_eq!(results[index]["ok"], true);
            assert_eq!(results[index]["cleanup"]["closed"], true);
        }
        assert_eq!(client.closes.lock().unwrap().len(), 2);
        assert!(client.lanes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ref_action_forwards_authoritative_lane_generation_not_frame_sequence() {
        let client = Arc::new(FakeLaneClient::default());
        let mut lane = client.snapshot("default", BrowserIdentityMode::Primary);
        lane.ref_generation = 42;
        let lane_id = lane.lane_id.clone();
        client.lanes.lock().unwrap().push(lane);
        let facade = facade(Arc::clone(&client));

        let result = facade
            .execute(
                "click",
                &json!({
                    "lane_id": lane_id.as_str(),
                    "ref": "f7e3",
                }),
            )
            .await;

        assert!(!result.is_error, "{}", result.content);
        let operations = client.operations.lock().unwrap();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].1.ref_generation, Some(42));
        assert_ne!(
            operations[0].1.ref_generation,
            Some(7),
            "the f<frame-seq> prefix must not be treated as an observation generation"
        );
    }

    #[tokio::test]
    async fn authenticated_replica_crawl_navigation_remains_read_path() {
        let client = Arc::new(FakeLaneClient::default());
        let lane = client.snapshot("replica-crawl", BrowserIdentityMode::AuthenticatedReplica);
        client.lanes.lock().unwrap().push(lane.clone());
        let result = run_crawl_item(
            client.clone(),
            &lane,
            "https://example.test/replica".to_owned(),
            None,
            1,
        )
        .await;

        assert_eq!(result["ok"], true);
        let operations = client.operations.lock().unwrap();
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].1.kind, BrowserOperationKind::Crawl);
        assert_eq!(operations[0].1.action, "navigate");
        assert!(!operations[0].1.may_modify_identity);
        assert!(!operations[1].1.may_modify_identity);
    }
}
