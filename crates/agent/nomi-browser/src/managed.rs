//! Shared Browser Platform tool dispatcher.
//!
//! This is deliberately smaller than [`crate::BrowserTool`]: it owns no
//! browser engine, Chromium process, profile, or caller identity.  It accepts
//! only an already-bound [`BrowserLaneClient`] and therefore gives the native
//! Agent, Gateway, and ACP/stdio surfaces one owner-scoped Lane contract.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nomi_types::tool::{ToolImage, ToolResult};
use nomifun_browser_platform::{
    BrowserIdentityMode, BrowserLaneClient, BrowserLaneId, BrowserLaneSnapshot,
    BrowserOperation, BrowserOperationKind, BrowserOperationResult, BrowserPlatformError,
    BrowserPresentationIntent, CloseResult, LaneLifecycleState, OpenLaneOutcome,
};
use serde_json::{Value, json};

use crate::OUT_OF_BAND_CONFIRMED_KEY;
/// Fields whose authority belongs to the main process.  A caller may select an
/// owner-scoped `lane_id`, but it may never construct or override identity,
/// target ownership, epochs, cancellation, or resource routing. One shared
/// list covers every managed surface; see [`crate::TRUSTED_OWNER_INPUT_FIELDS`].
use crate::TRUSTED_OWNER_INPUT_FIELDS;

const MAX_CRAWL_CONCURRENCY: usize = 8;
const MAX_CRAWL_URLS: usize = 64;
const MAX_CRAWL_URL_BYTES: usize = 16 * 1024;
const MAX_CRAWL_URLS_RETAINED_BYTES: usize = 256 * 1024;
const MAX_CRAWL_ITEM_RETAINED_BYTES: usize = 9 * 1024 * 1024;
const MAX_CRAWL_BATCH_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const CRAWL_BATCH_ENVELOPE_RESERVE_BYTES: usize = 8 * 1024;
const CRAWL_RESULT_BASE_RESERVE_BYTES: usize = 40 * 1024;
const CRAWL_RESULT_POSTPROCESS_RESERVE_BYTES: usize = 4 * 1024;
const CRAWL_OPERATION_TIMEOUT: Duration = Duration::from_secs(45);
const CRAWL_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const CRAWL_CLEANUP_CLOSE_CONCURRENCY: usize = 4;
const CRAWL_CLEANUP_DISPATCHER_WORKERS: usize = 2;
const CRAWL_CLEANUP_MAX_AUTHORITIES: usize = 256;
const CRAWL_CLEANUP_MAX_AUTHORITIES_PER_RUNTIME: usize = 64;
const CRAWL_CLEANUP_RETRY_MIN: Duration = Duration::from_millis(50);
const CRAWL_CLEANUP_RETRY_MAX: Duration = Duration::from_secs(1);
const CRAWL_CLEANUP_EXACT_RETRY_ESCALATE: u32 = 3;
const INVALID_CRAWL_REQUEST_CODE: &str = "invalid_browser_request";
static MANAGED_LANE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Identity selection is trusted host policy, never model input. Keep this
/// separate from owner metadata so callers receive the stable
/// `invalid_browser_request` contract rather than an owner-spoofing error.
/// ONE shared list for every managed surface (the native [`crate::BrowserTool`]
/// managed path and this facade).
pub(crate) const MODEL_IDENTITY_INPUT_FIELDS: &[&str] = &[
    "identity",
    "identity_mode",
    "authenticated",
    "auth_identity",
    "profile",
    "account",
];

#[async_trait]
pub(crate) trait BrowserLaneClientPort: Send + Sync {
    /// Legacy ABI name for the exact runtime cleanup key sealed into this
    /// client. It is intentionally not the task resource-family key used for
    /// quotas: Drop/cleanup deduplication must never merge sibling runtimes.
    fn task_resource_key(&self) -> String;

    /// Synchronously transfer one exact Lane cleanup authority to the Hub.
    /// The Lane id is sealed by the successful `open` result; cleanup must
    /// never widen to every Lane in the runtime (or installation) when a local
    /// dispatcher is saturated or unavailable.
    fn handoff_bound_lane_cleanup(
        &self,
        lane_id: BrowserLaneId,
    ) -> Result<(), BrowserPlatformError>;

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

    /// Report the Agent's declared presentation intent for one Lane.
    ///
    /// Deliberately has **no default implementation**: a defaulted trait method
    /// silently inherited by every test fake has bitten this crate before (see
    /// the `profile_footprint` fail-closed incident in the 2026-08-04 hardening
    /// handoff). Each implementor states its own behaviour.
    ///
    /// Returns `()` rather than a snapshot because the call is advisory — the
    /// trusted host may decline, and the Agent's next `status`/`observe` reports
    /// the authoritative state either way.
    async fn apply_presentation_intent(
        &self,
        lane_id: &BrowserLaneId,
        intent: BrowserPresentationIntent,
    ) -> Result<(), BrowserPlatformError>;
}

#[async_trait]
impl BrowserLaneClientPort for BrowserLaneClient {
    fn task_resource_key(&self) -> String {
        BrowserLaneClient::task_resource_key(self)
    }

    fn handoff_bound_lane_cleanup(
        &self,
        lane_id: BrowserLaneId,
    ) -> Result<(), BrowserPlatformError> {
        BrowserLaneClient::handoff_bound_lane_cleanup(self, lane_id)
    }

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

    async fn apply_presentation_intent(
        &self,
        lane_id: &BrowserLaneId,
        intent: BrowserPresentationIntent,
    ) -> Result<(), BrowserPlatformError> {
        BrowserLaneClient::apply_presentation_intent(self, lane_id, intent)
            .await
            .map(|_| ())
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

        let canonical = canonical_browser_action(action);

        match canonical {
            "browser_open" => self.open(input, false).await,
            "browser_fork" => self.open(input, true).await,
            "browser_list" => list_lanes(self.client.as_ref()).await,
            "browser_status" => self.status(input).await,
            "browser_close" => self.close(input).await,
            "browser_close_all" => close_all_lanes(self.client.as_ref()).await,
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
        open_lane(
            self.client.as_ref(),
            self.workspace_dir.as_deref(),
            &MANAGED_LANE_SEQUENCE,
            input,
            fork,
            "Opening a browser Lane failed",
        )
        .await
    }

    async fn status(&self, input: &Value) -> ToolResult {
        let lane_id = match self.resolve_lane_id(input).await {
            Ok(Some(lane_id)) => lane_id,
            Ok(None) => {
                return invalid_browser_request(
                    "No default browser Lane exists. Run `browser_open` first.",
                );
            }
            Err(error) => return platform_error_result("Resolving the browser Lane failed", error),
        };
        lane_status(self.client.as_ref(), &lane_id).await
    }

    async fn close(&self, input: &Value) -> ToolResult {
        let lane_id = match self.resolve_lane_id(input).await {
            Ok(lane_id) => lane_id,
            Err(error) => return platform_error_result("Resolving the browser Lane failed", error),
        };
        close_lane(self.client.as_ref(), lane_id).await
    }

    async fn execute_existing(&self, action: &str, input: &Value) -> ToolResult {
        execute_existing_operation(
            self.client.as_ref(),
            self.workspace_dir.as_deref(),
            action,
            input,
            true,
        )
        .await
    }

    async fn resolve_lane_id(
        &self,
        input: &Value,
    ) -> Result<Option<BrowserLaneId>, BrowserPlatformError> {
        if let Some(lane_id) = managed_lane_id(input)? {
            // Authorization happens here before the handle is used.
            self.client.status(&lane_id).await?;
            return Ok(Some(lane_id));
        }
        Ok(find_default_lane(self.client.as_ref())
            .await?
            .map(|lane| lane.lane_id))
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

/// Shared open/fork dispatch for every managed surface.
///
/// Parse the Agent's declared presentation intent.
///
/// The model states *what kind of moment* this is; it may not name a mechanism.
/// `headless`/`headful`/`external`/`visible` are therefore rejected rather than
/// quietly accepted, so a model that guesses the wrong vocabulary gets told
/// instead of silently falling back to routine.
///
/// An absent field means routine work, which is the overwhelmingly common case.
pub(crate) fn parse_presentation_intent(
    input: &Value,
) -> Result<BrowserPresentationIntent, String> {
    let Some(raw) = input.get("presentation") else {
        return Ok(BrowserPresentationIntent::Unattended);
    };
    let Some(text) = raw.as_str() else {
        return Err(
            "Browser input field `presentation` must be the string \"attended\" or \
             \"unattended\"."
                .to_owned(),
        );
    };
    match text.trim().to_ascii_lowercase().as_str() {
        "attended" => Ok(BrowserPresentationIntent::Attended),
        "unattended" => Ok(BrowserPresentationIntent::Unattended),
        "headless" | "headful" | "external" | "visible" | "silent" | "background"
        | "foreground" => Err(format!(
            "Browser input field `presentation` selects trusted host visibility policy \
             and does not accept `{text}`. Declare the intent instead: \"attended\" when \
             the user may need to see or take over (a sign-in wall, a challenge, a \
             consequential confirmation), otherwise \"unattended\"."
        )),
        other => Err(format!(
            "Unsupported `presentation` value `{other}`. Use \"attended\" or \"unattended\"."
        )),
    }
}

/// Only the platform-error `context` differs per surface (the native tool
/// reports fork failures as "Forking a browser Lane failed"; the facade uses
/// one open context for both), so the caller resolves it. `lane_sequence`
/// stays caller-owned: the facade shares one process-wide counter while each
/// native `BrowserTool` names forks from its own counter.
pub(crate) async fn open_lane(
    client: &dyn BrowserLaneClientPort,
    workspace_dir: Option<&Path>,
    lane_sequence: &AtomicU64,
    input: &Value,
    fork: bool,
    context: &str,
) -> ToolResult {
    let generated_name;
    let lane_name = match input.get("lane_name").and_then(Value::as_str) {
        Some(name) => Some(name),
        None if fork => {
            let sequence = lane_sequence.fetch_add(1, Ordering::AcqRel) + 1;
            generated_name = format!("fork-{sequence}");
            Some(generated_name.as_str())
        }
        None => None,
    };
    let workspace_hint = workspace_dir.map(|path| path.to_string_lossy().into_owned());
    let intent = match parse_presentation_intent(input) {
        Ok(intent) => intent,
        Err(message) => return ToolResult::error(message),
    };
    match client
        // Model-facing open/fork always uses the trusted live Primary
        // identity. The model may choose only the logical lane name;
        // identity policy is resolved by the host/Hub.
        .open(lane_name, BrowserIdentityMode::Primary, workspace_hint)
        .await
    {
        Ok(outcome) => {
            let lane = outcome.lane();
            // Report the intent after the Lane exists, so the trusted host can
            // resolve visibility against a real Lane. It is advisory: a declined
            // escalation is not an open failure, and the Agent's work proceeds
            // silently, so a reporting error must not fail the open.
            if intent == BrowserPresentationIntent::Attended
                && let Err(error) = client
                    .apply_presentation_intent(&lane.lane_id, intent)
                    .await
            {
                tracing::debug!(
                    code = ?error.code,
                    "the declared browser presentation intent was not applied"
                );
            }
            ToolResult::text(pretty_json(&json!({
                "ok": true,
                "action": if fork { "browser_fork" } else { "browser_open" },
                "lane": public_lane_json(lane),
                "queued": matches!(outcome, OpenLaneOutcome::Queued { .. }),
                "next_action": lane_next_action(lane),
            })))
        }
        Err(error) => platform_error_result(context, error),
    }
}

/// Map the short model-facing aliases (`open`, `fork`, …) onto the canonical
/// `browser_*` management action names. Existing page-level actions pass
/// through unchanged. Shared by every managed dispatcher.
pub(crate) fn canonical_browser_action(action: &str) -> &str {
    match action {
        "open" => "browser_open",
        "fork" => "browser_fork",
        "list" => "browser_list",
        "status" => "browser_status",
        "close" => "browser_close",
        "close_all" => "browser_close_all",
        "crawl_many" => "browser_crawl_many",
        other => other,
    }
}

/// Shared `browser_list` dispatch for every managed surface.
pub(crate) async fn list_lanes(client: &dyn BrowserLaneClientPort) -> ToolResult {
    match client.list().await {
        Ok(lanes) => ToolResult::text(pretty_json(&json!({
            "ok": true,
            "action": "browser_list",
            "lanes": lanes.iter().map(public_lane_json).collect::<Vec<_>>(),
        }))),
        Err(error) => platform_error_result("Listing browser Lanes failed", error),
    }
}

/// Shared `browser_status` tail for every managed surface. Lane-id resolution
/// (and its per-surface error contexts / missing-default shapes) stays with
/// the caller; this renders the authoritative snapshot.
pub(crate) async fn lane_status(
    client: &dyn BrowserLaneClientPort,
    lane_id: &BrowserLaneId,
) -> ToolResult {
    match client.status(lane_id).await {
        Ok(lane) => ToolResult::text(pretty_json(&json!({
            "ok": true,
            "action": "browser_status",
            "lane": public_lane_json(&lane),
            "next_action": lane_next_action(&lane),
        }))),
        Err(error) => platform_error_result("Reading browser Lane status failed", error),
    }
}

/// Shared `browser_close` tail for every managed surface. `None` means no
/// Lane was resolved: closing nothing is a success (already closed).
pub(crate) async fn close_lane(
    client: &dyn BrowserLaneClientPort,
    lane_id: Option<BrowserLaneId>,
) -> ToolResult {
    let Some(lane_id) = lane_id else {
        return ToolResult::text(pretty_json(&json!({
            "ok": true,
            "action": "browser_close",
            "closed": 0,
            "already_closed": true,
        })));
    };
    match client.close(&lane_id).await {
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

/// Shared `browser_close_all` dispatch for every managed surface.
pub(crate) async fn close_all_lanes(client: &dyn BrowserLaneClientPort) -> ToolResult {
    match client.close_all().await {
        Ok(result) => ToolResult::text(pretty_json(&json!({
            "ok": true,
            "action": "browser_close_all",
            "closed": result.closed,
            "already_closed": result.already_closed,
        }))),
        Err(error) => platform_error_result("Closing browser Lanes failed", error),
    }
}

/// The owner-scoped Lane named `default`, if one exists.
pub(crate) async fn find_default_lane(
    client: &dyn BrowserLaneClientPort,
) -> Result<Option<BrowserLaneSnapshot>, BrowserPlatformError> {
    Ok(client
        .list()
        .await?
        .into_iter()
        .find(|lane| lane.lane_key.lane_name == "default"))
}

/// Shared dispatch of an existing (page-level) browser action for every
/// managed surface: resolve the target Lane (an explicit owner-scoped
/// `lane_id`, else open/reuse the default Lane), require it to be running,
/// sanitize the model input into a [`BrowserOperation`], execute, and render
/// the result envelope.
///
/// `merge_legacy_output` selects the envelope shape: the shared facade merges
/// the operation output's fields into the top level (legacy transport
/// compatibility) and marks screenshots `captured`; the native Agent tool
/// keeps the plain `output` envelope with no merge.
pub(crate) async fn execute_existing_operation(
    client: &dyn BrowserLaneClientPort,
    workspace_dir: Option<&Path>,
    action: &str,
    input: &Value,
    merge_legacy_output: bool,
) -> ToolResult {
    let lane = match resolve_running_lane(client, workspace_dir, input).await {
        Ok(lane) => lane,
        Err(error) => return platform_error_result("Resolving the browser Lane failed", error),
    };
    if lane.lifecycle_state != LaneLifecycleState::Running {
        return lane_operation_not_dispatched_result(action, &lane);
    }

    // An attended moment can be discovered mid-task — the Agent navigates, hits
    // a sign-in wall, and only then needs the user. Report it before dispatching
    // so the window is up by the time the user is asked to act. Advisory: the
    // trusted host may decline (a pinned-silent policy, a spent allowance), and
    // the operation proceeds either way.
    match parse_presentation_intent(input) {
        Ok(BrowserPresentationIntent::Attended) => {
            if let Err(error) = client
                .apply_presentation_intent(&lane.lane_id, BrowserPresentationIntent::Attended)
                .await
            {
                tracing::debug!(
                    code = ?error.code,
                    "the declared browser presentation intent was not applied"
                );
            }
        }
        Ok(BrowserPresentationIntent::Unattended) => {}
        Err(message) => return ToolResult::error(message),
    }

    let operation = BrowserOperation {
        kind: operation_kind(action),
        action: action.to_owned(),
        input: sanitize_operation_input(input),
        expected_browser_epoch: input
            .get("expected_browser_epoch")
            .or_else(|| input.get("browser_epoch"))
            .and_then(Value::as_u64),
        // Target/frame ownership is selected by the Lane driver. Model
        // input is never allowed to override these routing fields.
        target_id: None,
        frame_id: None,
        // `f<seq>e<n>` embeds a frame sequence, not the observation
        // generation. Bind ref-bearing operations to the authoritative
        // Lane snapshot instead of parsing the model-visible ref text.
        ref_generation: input
            .get("ref")
            .and_then(Value::as_str)
            .is_some()
            .then_some(lane.ref_generation),
        may_modify_identity: action_may_modify_identity(action, input),
    };
    match client.execute(&lane.lane_id, operation).await {
        Ok(result) => {
            let latest = client.status(&lane.lane_id).await.ok();
            operation_result(
                action,
                &lane.lane_id,
                result,
                latest.as_ref(),
                merge_legacy_output,
            )
        }
        Err(error) => platform_error_result("Browser Lane operation failed", error),
    }
}

async fn resolve_running_lane(
    client: &dyn BrowserLaneClientPort,
    workspace_dir: Option<&Path>,
    input: &Value,
) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
    if let Some(lane_id) = managed_lane_id(input)? {
        return client.status(&lane_id).await;
    }
    let workspace_hint = workspace_dir.map(|path| path.to_string_lossy().into_owned());
    client
        .open(None, BrowserIdentityMode::Primary, workspace_hint)
        .await
        .map(|outcome| outcome.lane().clone())
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

/// Shared retained-byte ledger for one `browser_crawl_many` call.
///
/// Every input URL owns a small fixed reserve for a compact terminal result;
/// larger results atomically consume the remaining batch allowance as soon as
/// a worker produces them.  Therefore concurrent workers never accumulate 64
/// individually-valid multi-megabyte `Value`s before the coordinator can
/// notice the aggregate overflow.
struct CrawlBatchRetainedBudget {
    extra_remaining: AtomicUsize,
}

impl CrawlBatchRetainedBudget {
    fn new(url_count: usize) -> Self {
        let results_budget = MAX_CRAWL_BATCH_OUTPUT_BYTES
            .saturating_sub(CRAWL_BATCH_ENVELOPE_RESERVE_BYTES);
        let base = CRAWL_RESULT_BASE_RESERVE_BYTES.saturating_mul(url_count);
        Self {
            extra_remaining: AtomicUsize::new(results_budget.saturating_sub(base)),
        }
    }

    fn retain(&self, result: Value) -> Value {
        let measured = nomi_browser_engine::actions::serialized_json_bytes_at_most(
            &result,
            MAX_CRAWL_ITEM_RETAINED_BYTES,
        );
        let Ok(bytes) = measured else {
            return compact_crawl_byte_limit_result(&result, "crawl_item_byte_limit");
        };
        let charged = bytes.saturating_add(CRAWL_RESULT_POSTPROCESS_RESERVE_BYTES);
        if charged <= CRAWL_RESULT_BASE_RESERVE_BYTES {
            return result;
        }
        let additional = charged - CRAWL_RESULT_BASE_RESERVE_BYTES;
        let admitted = self
            .extra_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(additional)
            })
            .is_ok();
        if admitted {
            result
        } else {
            compact_crawl_byte_limit_result(&result, "crawl_batch_byte_limit")
        }
    }
}

fn compact_crawl_byte_limit_result(result: &Value, code: &'static str) -> Value {
    let compact = json!({
        "url": result.get("url").and_then(Value::as_str).unwrap_or_default(),
        "ok": false,
        "lane_id": result.get("lane_id").cloned().unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": "The crawl result exceeded this batch's retained-byte limit.",
            "retryable": true,
            "next_action": "Retry this URL alone or request a smaller extraction.",
        }
    });
    debug_assert!(
        nomi_browser_engine::actions::serialized_json_bytes_at_most(
            &compact,
            CRAWL_RESULT_BASE_RESERVE_BYTES,
        )
        .is_ok(),
        "compact crawl byte-limit result must fit its fixed reserve"
    );
    compact
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

#[derive(Clone, Copy)]
struct CrawlCleanupDispatcherLimits {
    max_authorities: usize,
    max_authorities_per_runtime: usize,
    workers: usize,
}

impl CrawlCleanupDispatcherLimits {
    const fn production() -> Self {
        Self {
            max_authorities: CRAWL_CLEANUP_MAX_AUTHORITIES,
            max_authorities_per_runtime: CRAWL_CLEANUP_MAX_AUTHORITIES_PER_RUNTIME,
            workers: CRAWL_CLEANUP_DISPATCHER_WORKERS,
        }
    }
}

#[derive(Clone)]
struct CrawlCleanupSubmission {
    lane_id: BrowserLaneId,
    lane_name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CrawlCleanupIntentStatus {
    Queued,
    InFlight,
}

struct CrawlCleanupIntent {
    task_resource_key: String,
    client: Arc<dyn BrowserLaneClientPort>,
    lane_name: String,
    lane_ids: HashSet<BrowserLaneId>,
    due_at: Instant,
    attempts: u32,
    status: CrawlCleanupIntentStatus,
}

impl CrawlCleanupIntent {
    fn authority_count(&self) -> usize {
        self.lane_ids.len()
    }

    fn is_empty(&self) -> bool {
        self.lane_ids.is_empty()
    }
}

#[derive(Default)]
struct CrawlCleanupDispatcherState {
    next_intent_id: u64,
    intents: HashMap<u64, CrawlCleanupIntent>,
    active_workers: usize,
    max_active_workers: usize,
    completed_authorities: usize,
    shutdown: bool,
}

impl CrawlCleanupDispatcherState {
    fn authority_count(&self) -> usize {
        self.intents
            .values()
            .map(CrawlCleanupIntent::authority_count)
            .sum()
    }

    fn runtime_authority_count(&self, task_resource_key: &str) -> usize {
        self.intents
            .values()
            .filter(|intent| intent.task_resource_key == task_resource_key)
            .map(CrawlCleanupIntent::authority_count)
            .sum()
    }
}

struct CrawlCleanupDispatcherInner {
    limits: CrawlCleanupDispatcherLimits,
    state: StdMutex<CrawlCleanupDispatcherState>,
    work_available: Condvar,
    live_workers: AtomicUsize,
}

#[derive(Clone)]
struct CrawlCleanupDispatcher {
    inner: Arc<CrawlCleanupDispatcherInner>,
    threads: Arc<StdMutex<Vec<std::thread::JoinHandle<()>>>>,
}

static CRAWL_CLEANUP_DISPATCHER: OnceLock<Option<CrawlCleanupDispatcher>> = OnceLock::new();

fn handoff_exact_crawl_lane(
    client: &Arc<dyn BrowserLaneClientPort>,
    lane_id: BrowserLaneId,
) -> bool {
    // Each admitted live Lane carries a Hub cleanup-ledger reservation, so a
    // valid exact handoff cannot fail because the ledger is full. An error is
    // stale/mismatched authority or an internal invariant violation; neither
    // permits widening cleanup to a runtime or installation snapshot.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.handoff_bound_lane_cleanup(lane_id.clone())
    })) {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::error!(
                lane_id = %lane_id,
                code = ?error.code,
                "Hub rejected an exact crawl Lane cleanup handoff"
            );
            false
        }
        Err(_) => {
            tracing::error!(
                lane_id = %lane_id,
                "exact crawl Lane cleanup handoff panicked"
            );
            false
        }
    }
}

impl CrawlCleanupDispatcher {
    fn global() -> Option<Self> {
        CRAWL_CLEANUP_DISPATCHER
            .get_or_init(|| match Self::new(CrawlCleanupDispatcherLimits::production()) {
                Ok(dispatcher) => Some(dispatcher),
                Err(error) => {
                    tracing::error!(
                        %error,
                        "managed crawl cleanup dispatcher unavailable; using exact Hub cleanup handoff"
                    );
                    None
                }
            })
            .clone()
    }

    fn new(limits: CrawlCleanupDispatcherLimits) -> Result<Self, String> {
        if limits.max_authorities == 0
            || limits.max_authorities_per_runtime == 0
            || limits.workers == 0
        {
            return Err("managed crawl cleanup dispatcher limits must be non-zero".to_owned());
        }
        let dispatcher = Self {
            inner: Arc::new(CrawlCleanupDispatcherInner {
                limits,
                state: StdMutex::new(CrawlCleanupDispatcherState::default()),
                work_available: Condvar::new(),
                live_workers: AtomicUsize::new(0),
            }),
            threads: Arc::new(StdMutex::new(Vec::with_capacity(limits.workers))),
        };
        for worker_index in 0..limits.workers {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!(
                        worker_index,
                        %error,
                        "failed to build a managed crawl cleanup runtime"
                    );
                    continue;
                }
            };
            let inner = Arc::clone(&dispatcher.inner);
            dispatcher.inner.live_workers.fetch_add(1, Ordering::AcqRel);
            match std::thread::Builder::new()
                .name(format!("nomi-crawl-cleanup-{worker_index}"))
                .spawn(move || crawl_cleanup_dispatcher_worker(inner, runtime))
            {
                Ok(thread) => dispatcher
                    .threads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(thread),
                Err(error) => {
                    dispatcher
                        .inner
                        .live_workers
                        .fetch_sub(1, Ordering::AcqRel);
                    tracing::error!(
                        worker_index,
                        %error,
                        "failed to start a managed crawl cleanup worker"
                    );
                }
            }
        }
        if dispatcher
            .threads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
        {
            Err("no managed crawl cleanup worker could be started".to_owned())
        } else {
            Ok(dispatcher)
        }
    }

    /// Synchronous, bounded ownership transfer used from Drop. Duplicate exact
    /// Lane authorities merge locally. If either local bound is full, the
    /// overflowing Lane is handed directly to the Hub's exact-Lane ledger;
    /// cleanup scope is never widened to the runtime or installation.
    fn enqueue_batch(
        &self,
        client: Arc<dyn BrowserLaneClientPort>,
        submissions: Vec<CrawlCleanupSubmission>,
    ) -> bool {
        if self.inner.live_workers.load(Ordering::Acquire) == 0 {
            return false;
        }
        let task_resource_key = client.task_resource_key();
        for submission in submissions {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                if state.shutdown || self.inner.live_workers.load(Ordering::Acquire) == 0 {
                    drop(state);
                    let _ = handoff_exact_crawl_lane(&client, submission.lane_id.clone());
                    break;
                }
                let matching_intent = state.intents.iter().find_map(|(intent_id, intent)| {
                    if intent.task_resource_key != task_resource_key {
                        return None;
                    }
                    let same_id = intent.lane_ids.contains(&submission.lane_id);
                    (same_id || intent.lane_name == submission.lane_name).then_some(*intent_id)
                });

                let additional_authorities = matching_intent
                    .and_then(|intent_id| state.intents.get(&intent_id))
                    .map(|intent| usize::from(!intent.lane_ids.contains(&submission.lane_id)))
                    .unwrap_or(1);
                if additional_authorities == 0 {
                    break;
                }

                let global_available = state
                    .authority_count()
                    .saturating_add(additional_authorities)
                    <= self.inner.limits.max_authorities;
                let client_available = state
                    .runtime_authority_count(&task_resource_key)
                    .saturating_add(additional_authorities)
                    <= self.inner.limits.max_authorities_per_runtime;
                if !global_available || !client_available {
                    drop(state);
                    let _ = handoff_exact_crawl_lane(&client, submission.lane_id.clone());
                    break;
                }

                let now = Instant::now();
                if let Some(intent_id) = matching_intent {
                    let intent = state
                        .intents
                        .get_mut(&intent_id)
                        .expect("matching cleanup intent remains present");
                    let changed = intent.lane_ids.insert(submission.lane_id.clone());
                    if changed {
                        intent.due_at = now;
                    }
                } else {
                    state.next_intent_id = state.next_intent_id.wrapping_add(1).max(1);
                    let intent_id = state.next_intent_id;
                    let lane_ids = HashSet::from([submission.lane_id.clone()]);
                    state.intents.insert(
                        intent_id,
                        CrawlCleanupIntent {
                            task_resource_key: task_resource_key.clone(),
                            client: Arc::clone(&client),
                            lane_name: submission.lane_name.clone(),
                            lane_ids,
                            due_at: now,
                            attempts: 0,
                            status: CrawlCleanupIntentStatus::Queued,
                        },
                    );
                }
                self.inner.work_available.notify_all();
                break;
            }
        }
        true
    }

    #[cfg(test)]
    fn snapshot(&self) -> CrawlCleanupDispatcherSnapshot {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CrawlCleanupDispatcherSnapshot {
            retained_authorities: state.authority_count(),
            retained_intents: state.intents.len(),
            active_workers: state.active_workers,
            max_active_workers: state.max_active_workers,
            completed_authorities: state.completed_authorities,
            live_workers: self.inner.live_workers.load(Ordering::Acquire),
        }
    }

    #[cfg(test)]
    fn shutdown_and_join(&self) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(
                state.intents.is_empty() && state.active_workers == 0,
                "test dispatcher must be idle before shutdown"
            );
            state.shutdown = true;
        }
        self.inner.work_available.notify_all();
        let threads = std::mem::take(
            &mut *self
                .threads
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for thread in threads {
            thread.join().expect("crawl cleanup worker thread joins");
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct CrawlCleanupDispatcherSnapshot {
    retained_authorities: usize,
    retained_intents: usize,
    active_workers: usize,
    max_active_workers: usize,
    completed_authorities: usize,
    live_workers: usize,
}

struct CrawlCleanupWork {
    intent_id: u64,
    client: Arc<dyn BrowserLaneClientPort>,
    lane_name: String,
    lane_id: BrowserLaneId,
    attempts: u32,
}

enum CrawlCleanupWorkOutcome {
    ExactClosed(BrowserLaneId),
    Retry,
}

fn crawl_cleanup_dispatcher_worker(
    inner: Arc<CrawlCleanupDispatcherInner>,
    runtime: tokio::runtime::Runtime,
) {
    let _liveness = CrawlCleanupWorkerLiveness {
        inner: Arc::clone(&inner),
    };
    while let Some(work) = take_crawl_cleanup_work(&inner) {
        let completion = CrawlCleanupWorkCompletion::new(&inner, &work);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(process_crawl_cleanup_work(&inner, &work))
        }))
        .unwrap_or_else(|_| {
            tracing::error!(
                lane_name = %work.lane_name,
                lane_id = %work.lane_id,
                "managed crawl cleanup client panicked; retaining authority for retry"
            );
            CrawlCleanupWorkOutcome::Retry
        });
        completion.finish(outcome);
    }
}

struct CrawlCleanupWorkCompletion<'a> {
    inner: &'a CrawlCleanupDispatcherInner,
    work: &'a CrawlCleanupWork,
    finished: bool,
}

impl<'a> CrawlCleanupWorkCompletion<'a> {
    fn new(inner: &'a CrawlCleanupDispatcherInner, work: &'a CrawlCleanupWork) -> Self {
        Self {
            inner,
            work,
            finished: false,
        }
    }

    fn finish(mut self, outcome: CrawlCleanupWorkOutcome) {
        // Set first so a panic inside bookkeeping cannot double-complete the
        // same authority while unwinding.
        self.finished = true;
        finish_crawl_cleanup_work(self.inner, self.work, outcome);
    }
}

impl Drop for CrawlCleanupWorkCompletion<'_> {
    fn drop(&mut self) {
        if !self.finished {
            finish_crawl_cleanup_work(
                self.inner,
                self.work,
                CrawlCleanupWorkOutcome::Retry,
            );
        }
    }
}

struct CrawlCleanupWorkerLiveness {
    inner: Arc<CrawlCleanupDispatcherInner>,
}

impl Drop for CrawlCleanupWorkerLiveness {
    fn drop(&mut self) {
        let previous = self.inner.live_workers.fetch_sub(1, Ordering::AcqRel);
        if previous != 1 {
            return;
        }

        // A dispatcher with no live workers must never keep claiming cleanup
        // authority. Transfer every retained exact Lane to the Hub ledger;
        // widening this to a task/global snapshot would be ABA-unsafe.
        let exact_authorities = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let authorities = state
                .intents
                .values()
                .flat_map(|intent| {
                    intent
                        .lane_ids
                        .iter()
                        .cloned()
                        .map(|lane_id| (Arc::clone(&intent.client), lane_id))
                })
                .collect::<Vec<_>>();
            state.intents.clear();
            state.active_workers = 0;
            authorities
        };
        self.inner.work_available.notify_all();
        for (client, lane_id) in exact_authorities {
            let _ = handoff_exact_crawl_lane(&client, lane_id);
        }
    }
}

fn take_crawl_cleanup_work(
    inner: &CrawlCleanupDispatcherInner,
) -> Option<CrawlCleanupWork> {
    let mut state = inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if state.shutdown && state.intents.is_empty() {
            return None;
        }
        let now = Instant::now();
        let candidate = state
            .intents
            .iter()
            .filter(|(_, intent)| intent.status == CrawlCleanupIntentStatus::Queued)
            .min_by_key(|(intent_id, intent)| (intent.due_at, **intent_id))
            .map(|(intent_id, intent)| (*intent_id, intent.due_at));
        match candidate {
            Some((intent_id, due_at)) if due_at <= now => {
                let intent = state
                    .intents
                    .get_mut(&intent_id)
                    .expect("selected cleanup intent remains present");
                intent.status = CrawlCleanupIntentStatus::InFlight;
                let lane_id = intent
                    .lane_ids
                    .iter()
                    .min_by(|left, right| left.as_str().cmp(right.as_str()))
                    .cloned()
                    .expect("non-empty cleanup intent has an exact Lane id");
                let work = CrawlCleanupWork {
                    intent_id,
                    client: Arc::clone(&intent.client),
                    lane_name: intent.lane_name.clone(),
                    lane_id,
                    attempts: intent.attempts,
                };
                state.active_workers += 1;
                state.max_active_workers = state.max_active_workers.max(state.active_workers);
                return Some(work);
            }
            Some((_, due_at)) => {
                let wait = due_at.saturating_duration_since(now);
                let (next_state, _) = inner
                    .work_available
                    .wait_timeout(state, wait)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state = next_state;
            }
            None => {
                state = inner
                    .work_available
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
    }
}

async fn process_crawl_cleanup_work(
    _inner: &CrawlCleanupDispatcherInner,
    work: &CrawlCleanupWork,
) -> CrawlCleanupWorkOutcome {
    let lane_id = &work.lane_id;
    match tokio::time::timeout(CRAWL_CLEANUP_TIMEOUT, work.client.close(lane_id)).await {
            Ok(Ok(_)) => CrawlCleanupWorkOutcome::ExactClosed(lane_id.clone()),
            Ok(Err(error))
                if error.code == nomifun_browser_platform::BrowserErrorCode::LaneNotFound =>
            {
                CrawlCleanupWorkOutcome::ExactClosed(lane_id.clone())
            }
            Ok(Err(error)) => {
                if should_log_cleanup_retry(work.attempts) {
                    tracing::warn!(
                        lane_id = %lane_id,
                        code = ?error.code,
                        "managed crawl cleanup dispatcher will retry Lane close"
                    );
                }
                CrawlCleanupWorkOutcome::Retry
            }
            Err(_) => {
                if should_log_cleanup_retry(work.attempts) {
                    tracing::warn!(
                        lane_id = %lane_id,
                        "managed crawl cleanup dispatcher timed out; retrying"
                    );
                }
                CrawlCleanupWorkOutcome::Retry
            }
    }
}

fn finish_crawl_cleanup_work(
    inner: &CrawlCleanupDispatcherInner,
    work: &CrawlCleanupWork,
    outcome: CrawlCleanupWorkOutcome,
) {
    let mut state = inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.active_workers = state.active_workers.saturating_sub(1);
    let mut completed = 0usize;
    let mut remove_intent = false;
    if let Some(intent) = state.intents.get_mut(&work.intent_id) {
        match outcome {
            CrawlCleanupWorkOutcome::ExactClosed(lane_id) => {
                if intent.lane_ids.remove(&lane_id) {
                    completed += 1;
                }
                intent.attempts = 0;
                intent.due_at = Instant::now();
            }
            CrawlCleanupWorkOutcome::Retry => {
                intent.attempts = intent.attempts.saturating_add(1);
                if intent.attempts >= CRAWL_CLEANUP_EXACT_RETRY_ESCALATE {
                    if handoff_exact_crawl_lane(&intent.client, work.lane_id.clone())
                        && intent.lane_ids.remove(&work.lane_id)
                    {
                        completed += 1;
                    }
                    if intent.lane_ids.contains(&work.lane_id) {
                        intent.due_at =
                            Instant::now() + crawl_cleanup_retry_delay(intent.attempts);
                    } else {
                        intent.attempts = 0;
                        intent.due_at = Instant::now();
                    }
                } else {
                    intent.due_at = Instant::now() + crawl_cleanup_retry_delay(intent.attempts);
                }
            }
        }
        remove_intent = intent.is_empty();
        if !remove_intent {
            intent.status = CrawlCleanupIntentStatus::Queued;
        }
    }
    if remove_intent {
        state.intents.remove(&work.intent_id);
    }
    state.completed_authorities = state.completed_authorities.saturating_add(completed);
    drop(state);
    inner.work_available.notify_all();
}

fn crawl_cleanup_retry_delay(attempt: u32) -> Duration {
    let factor = 1u32 << attempt.min(4);
    CRAWL_CLEANUP_RETRY_MIN
        .saturating_mul(factor)
        .min(CRAWL_CLEANUP_RETRY_MAX)
}

fn should_log_cleanup_retry(attempt: u32) -> bool {
    attempt == 0 || attempt.is_power_of_two()
}


/// Owns only concrete Lane IDs returned by this batch's successful opens.
/// Cancellation while `open` itself is pending is already covered by the
/// Hub's exact abandoned-start authority; re-resolving by name here would
/// create a second, ABA-prone cleanup owner.
struct CrawlBatchCleanup {
    client: Arc<dyn BrowserLaneClientPort>,
    owned_lanes: HashMap<BrowserLaneId, String>,
    dispatcher: Option<CrawlCleanupDispatcher>,
    allow_global_dispatcher: bool,
}

impl CrawlBatchCleanup {
    fn new(client: Arc<dyn BrowserLaneClientPort>) -> Self {
        Self {
            client,
            owned_lanes: HashMap::new(),
            // Lazily initialize the process singleton only if Drop actually
            // has residual authority to hand off. Successful batches pay no
            // persistent worker-thread cost.
            dispatcher: None,
            allow_global_dispatcher: true,
        }
    }

    #[cfg(test)]
    fn new_with_dispatcher(
        client: Arc<dyn BrowserLaneClientPort>,
        dispatcher: CrawlCleanupDispatcher,
    ) -> Self {
        Self {
            client,
            owned_lanes: HashMap::new(),
            dispatcher: Some(dispatcher),
            allow_global_dispatcher: true,
        }
    }

    #[cfg(test)]
    fn new_with_unavailable_dispatcher(client: Arc<dyn BrowserLaneClientPort>) -> Self {
        Self {
            client,
            owned_lanes: HashMap::new(),
            dispatcher: None,
            allow_global_dispatcher: false,
        }
    }

    fn track_owned_lane(&mut self, lane: &BrowserLaneSnapshot) {
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
        if self.owned_lanes.is_empty() {
            return;
        }
        let client = Arc::clone(&self.client);
        let mut submissions = std::mem::take(&mut self.owned_lanes)
            .into_iter()
            .map(|(lane_id, lane_name)| CrawlCleanupSubmission {
                lane_id,
                lane_name,
            })
            .collect::<Vec<_>>();
        submissions.sort_by(|left, right| {
            left
                .lane_name
                .cmp(&right.lane_name)
                .then_with(|| left.lane_id.as_str().cmp(right.lane_id.as_str()))
        });
        let dispatcher = self.dispatcher.take().or_else(|| {
            self.allow_global_dispatcher
                .then(CrawlCleanupDispatcher::global)
                .flatten()
        });
        if let Some(dispatcher) = dispatcher {
            let exact_fallback = submissions.clone();
            if dispatcher.enqueue_batch(Arc::clone(&client), submissions) {
                return;
            }
            submissions = exact_fallback;
        }
        for submission in submissions {
            let _ = handoff_exact_crawl_lane(&client, submission.lane_id);
        }
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
    let retained_budget = Arc::new(CrawlBatchRetainedBudget::new(request.urls.len()));
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
                            retained_budget.retain(crawl_error_result(
                                url,
                                None,
                                request.identity_mode,
                                request.requested_concurrency,
                                public_platform_error_json(&error),
                            ))
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
                let result = match terminal_lanes.get(&worker_index) {
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
                };
                retained_budget.retain(result)
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
            Arc::clone(&retained_budget),
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
                        retained_budget.retain(crawl_worker_terminal_failure(
                            url,
                            &plan.lane,
                            plan.recommended_concurrency,
                            "worker_incomplete",
                        ))
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
                    ordered_results[*index] = Some(retained_budget.retain(
                        crawl_worker_terminal_failure(
                        url,
                        &plan.lane,
                        effective_concurrency,
                        cause,
                    )));
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
            ordered_results[index] = Some(retained_budget.retain(crawl_error_result(
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
            )));
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
    retained_budget: Arc<CrawlBatchRetainedBudget>,
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
        results.push((index, retained_budget.retain(result)));
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
    let envelope = json!({
        "ok": results.iter().all(|result| {
            result.get("ok").and_then(Value::as_bool).unwrap_or(false)
        }),
        "action": "browser_crawl_many",
        "identity_mode": request.identity_mode,
        "requested": request.urls.len(),
        "concurrency": effective_concurrency,
        "results": results,
    });
    let Ok(bytes) = nomi_browser_engine::actions::serialized_json_bytes_at_most(
        &envelope,
        MAX_CRAWL_BATCH_OUTPUT_BYTES,
    ) else {
        return ToolResult::error(
            "{\"ok\":false,\"action\":\"browser_crawl_many\",\"error\":{\"code\":\"crawl_batch_byte_limit\",\"message\":\"The crawl batch exceeded its retained-byte limit. Retry fewer URLs or a smaller extraction.\"}}"
                .to_owned(),
        );
    };
    // Compact JSON avoids the otherwise-unbounded pretty-print whitespace
    // copy. The envelope remains alive during serialization, but both copies
    // now have the same deterministic per-batch ceiling.
    let mut encoded = Vec::with_capacity(bytes);
    if serde_json::to_writer(&mut encoded, &envelope).is_err() {
        return ToolResult::error(
            "{\"ok\":false,\"action\":\"browser_crawl_many\",\"error\":{\"code\":\"crawl_batch_serialization_failed\",\"message\":\"The bounded crawl result could not be serialized.\"}}"
                .to_owned(),
        );
    }
    match String::from_utf8(encoded) {
        Ok(encoded) => ToolResult::text(encoded),
        Err(_) => ToolResult::error(
            "{\"ok\":false,\"action\":\"browser_crawl_many\",\"error\":{\"code\":\"crawl_batch_serialization_failed\",\"message\":\"The bounded crawl result was not UTF-8.\"}}"
                .to_owned(),
        ),
    }
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
    let mut seen = HashSet::new();
    let mut pending = lane_ids
        .into_iter()
        .filter(|lane_id| seen.insert(lane_id.clone()))
        .collect::<Vec<_>>();
    let mut workers = tokio::task::JoinSet::new();
    let mut task_lanes = HashMap::new();
    while workers.len() < CRAWL_CLEANUP_CLOSE_CONCURRENCY {
        let Some(lane_id) = pending.pop() else {
            break;
        };
        spawn_crawl_close_worker(&mut workers, &mut task_lanes, Arc::clone(&client), lane_id);
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
        while workers.len() < CRAWL_CLEANUP_CLOSE_CONCURRENCY {
            let Some(lane_id) = pending.pop() else {
                break;
            };
            spawn_crawl_close_worker(
                &mut workers,
                &mut task_lanes,
                Arc::clone(&client),
                lane_id,
            );
        }
    }
    workers.abort_all();
    for lane_id in task_lanes.into_values().chain(pending) {
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

fn spawn_crawl_close_worker(
    workers: &mut tokio::task::JoinSet<Result<CloseResult, BrowserPlatformError>>,
    task_lanes: &mut HashMap<tokio::task::Id, BrowserLaneId>,
    client: Arc<dyn BrowserLaneClientPort>,
    lane_id: BrowserLaneId,
) {
    let worker_lane_id = lane_id.clone();
    let abort_handle = workers.spawn(async move { client.close(&worker_lane_id).await });
    task_lanes.insert(abort_handle.id(), lane_id);
}

pub(crate) fn managed_lane_id(input: &Value) -> Result<Option<BrowserLaneId>, BrowserPlatformError> {
    // F55: a typed-wrong lane_id (number/bool/object) is an explicit error,
    // never a silent fallback to the caller's default Lane (which could
    // execute the action against the wrong logged-in page).
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
    let schema = crawl_schema(input)?;
    Ok(ManagedCrawlRequest {
        urls,
        requested_concurrency,
        auto_concurrency: crawl_concurrency_is_auto(input),
        identity_mode: BrowserIdentityMode::Anonymous,
        schema,
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
    let mut urls = Vec::with_capacity(values.len());
    let mut retained_bytes = 0usize;
    for (index, value) in values.iter().enumerate() {
        let raw = value
            .as_str()
            .ok_or_else(|| format!("`urls[{index}]` must be a string."))?;
        if raw.len() > MAX_CRAWL_URL_BYTES {
            return Err(format!(
                "`urls[{index}]` exceeds the {MAX_CRAWL_URL_BYTES}-byte URL limit."
            ));
        }
        let url = raw.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("`urls[{index}]` must be an HTTP(S) URL."));
        }
        if url.chars().any(|character| character.is_control()) {
            return Err(format!("`urls[{index}]` contains a control character."));
        }
        retained_bytes = retained_bytes
            .checked_add(url.len())
            .ok_or_else(|| "`urls` retained-byte count overflowed.".to_owned())?;
        if retained_bytes > MAX_CRAWL_URLS_RETAINED_BYTES {
            return Err(format!(
                "`urls` exceeds the {MAX_CRAWL_URLS_RETAINED_BYTES}-byte aggregate URL limit."
            ));
        }
        urls.push(url.to_owned());
    }
    Ok(urls)
}

fn crawl_schema(input: &Value) -> Result<Option<Value>, String> {
    let Some(schema) = input.get("schema") else {
        return Ok(None);
    };
    if schema.is_null() {
        return Ok(None);
    }
    if !(schema.is_object() || schema.is_array()) {
        return Err("`schema` must be a JSON object or array when provided.".to_owned());
    }
    if let Err(error) =
        nomi_browser_engine::actions::validate_extract_schema_capacity(schema)
    {
        return Err(format!(
            "`schema` exceeds the extraction-schema capacity ({error:?}; bytes={}, depth={}, nodes={}).",
            nomi_browser_engine::actions::MAX_EXTRACT_SCHEMA_BYTES,
            nomi_browser_engine::actions::MAX_EXTRACT_SCHEMA_DEPTH,
            nomi_browser_engine::actions::MAX_EXTRACT_SCHEMA_NODES,
        ));
    }
    Ok(Some(schema.clone()))
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

pub(crate) fn first_model_identity_field(input: &Value) -> Option<&'static str> {
    let object = input.as_object()?;
    MODEL_IDENTITY_INPUT_FIELDS
        .iter()
        .copied()
        .find(|field| object.contains_key(*field))
}

pub(crate) fn first_trusted_owner_field(input: &Value) -> Option<&'static str> {
    let object = input.as_object()?;
    TRUSTED_OWNER_INPUT_FIELDS
        .iter()
        .copied()
        .find(|field| object.contains_key(*field))
}

pub(crate) fn sanitize_operation_input(input: &Value) -> Value {
    let mut sanitized = input.as_object().cloned().unwrap_or_default();
    sanitized.remove("lane_id");
    sanitized.remove("lane_name");
    sanitized.remove("expected_browser_epoch");
    // Host visibility policy, consumed by `execute_existing_operation` before
    // dispatch. It is not an action parameter and must not reach the driver.
    sanitized.remove("presentation");
    sanitized.remove(OUT_OF_BAND_CONFIRMED_KEY);
    for field in TRUSTED_OWNER_INPUT_FIELDS {
        sanitized.remove(*field);
    }
    Value::Object(sanitized)
}

pub(crate) fn is_existing_browser_action(action: &str) -> bool {
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

pub(crate) fn operation_kind(action: &str) -> BrowserOperationKind {
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

pub(crate) fn action_may_modify_identity(action: &str, input: &Value) -> bool {
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

pub(crate) fn lane_operation_not_dispatched_result(
    action: &str,
    lane: &BrowserLaneSnapshot,
) -> ToolResult {
    let (code, message, retryable) = match lane.lifecycle_state {
        LaneLifecycleState::Queued => (
            json!("browser_capacity_queued"),
            "The browser Lane is queued, so the page operation was not dispatched.",
            true,
        ),
        LaneLifecycleState::Starting => (
            json!("browser_unavailable"),
            "The browser Lane is still starting, so the page operation was not dispatched.",
            true,
        ),
        LaneLifecycleState::Frozen => (
            json!("browser_unavailable"),
            "The browser Lane is frozen by resource pressure, so the page operation was not dispatched.",
            true,
        ),
        LaneLifecycleState::Stopping => (
            json!("browser_unavailable"),
            "The browser Lane is stopping, so the page operation was not dispatched.",
            false,
        ),
        LaneLifecycleState::Failed => (
            lane.error_code
                .as_ref()
                .map_or_else(|| json!("browser_unavailable"), |code| json!(code)),
            lane.error_message
                .as_deref()
                .unwrap_or("The browser Lane failed before the page operation could run."),
            lane.recoverable,
        ),
        LaneLifecycleState::Running => (
            json!("browser_unavailable"),
            "The browser Lane was not ready for the page operation.",
            true,
        ),
    };
    let next_action = lane_next_action(lane);
    let retry_after_ms = lane.queue.as_ref().map(|queue| queue.retry_delay_ms);
    let reason_code = lane
        .queue
        .as_ref()
        .map(|queue| queue.reason_code.as_str());
    ToolResult::error(pretty_json(&json!({
        "ok": false,
        "action": action,
        "dispatched": false,
        "error_code": code.clone(),
        "retryable": retryable,
        "retry_after_ms": retry_after_ms,
        "lane": public_lane_json(lane),
        "error": {
            "code": code,
            "message": message,
            "retryable": retryable,
            "retry_after_ms": retry_after_ms,
            "next_action": next_action,
            "metadata": {
                "lifecycle_state": lane.lifecycle_state,
                "reason_code": reason_code,
                "retry_delay_ms": retry_after_ms,
            },
        },
        "next_action": next_action,
    })))
}

pub(crate) fn lane_next_action(lane: &BrowserLaneSnapshot) -> &'static str {
    match lane.lifecycle_state {
        LaneLifecycleState::Queued => {
            "After retry_delay_ms, call browser_status with this lane_id; do not use the page wait action while queued. Reuse a running Lane or lower concurrency."
        }
        LaneLifecycleState::Starting => {
            "Call browser_status with this lane_id after a short delay; retry the page operation only after the Lane is running."
        }
        LaneLifecycleState::Running => "Use the returned lane_id for browser operations.",
        LaneLifecycleState::Frozen => "Reuse an active Lane or wait for capacity to recover.",
        LaneLifecycleState::Stopping => "Open a replacement Lane only if more work is required.",
        LaneLifecycleState::Failed => {
            "Inspect error_code and recoverable; open a replacement Lane when advised."
        }
    }
}

/// Render an executed operation's result envelope. `merge_legacy_output`
/// additionally mirrors the output's fields at the top level and marks
/// screenshots `captured` — the loopback/Gateway transport shape. The native
/// Agent tool passes `false` and keeps the plain `output` envelope.
fn operation_result(
    action: &str,
    lane_id: &BrowserLaneId,
    result: BrowserOperationResult,
    lane: Option<&BrowserLaneSnapshot>,
    merge_legacy_output: bool,
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
    if merge_legacy_output && let Some(envelope) = envelope.as_object_mut() {
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

pub(crate) fn public_platform_error_json(error: &BrowserPlatformError) -> Value {
    json!({
        "code": error.code,
        "message": error.message,
        "retryable": error.retryable,
        "next_action": error.next_action,
        "lane_id": error.lane_id.as_ref().map(BrowserLaneId::as_str),
        "metadata": error.metadata,
    })
}

pub(crate) fn platform_error_result(context: &str, error: BrowserPlatformError) -> ToolResult {
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

pub(crate) fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_tools::Tool;
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    static FAKE_TASK_RESOURCE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct FakeLaneClient {
        task_resource_key: OnceLock<String>,
        opens: Mutex<Vec<(Option<String>, BrowserIdentityMode, Option<String>)>>,
        operations: Mutex<Vec<(BrowserLaneId, BrowserOperation)>>,
        closes: Mutex<Vec<BrowserLaneId>>,
        presentation_intents: Mutex<Vec<(BrowserLaneId, BrowserPresentationIntent)>>,
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
        block_close: AtomicBool,
        close_release: (Mutex<bool>, Condvar),
        close_active: AtomicUsize,
        close_max_active: AtomicUsize,
        close_all_calls: AtomicUsize,
        fail_close: AtomicBool,
        reject_public_close_all: AtomicBool,
        panic_close_remaining: AtomicUsize,
        panic_list_remaining: AtomicUsize,
        exact_handoffs: Mutex<Vec<BrowserLaneId>>,
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
                error_code: None,
                error_message: None,
                recoverable: false,
            }
        }

        fn release_all_closes(&self) {
            let (released, wake) = &self.close_release;
            *released.lock().unwrap() = true;
            wake.notify_all();
        }

        fn take_injected_panic(counter: &AtomicUsize) -> bool {
            counter
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    (remaining > 0).then(|| remaining - 1)
                })
                .is_ok()
        }

        fn apply_exact_handoffs(&self) {
            let handed_off = std::mem::take(&mut *self.exact_handoffs.lock().unwrap());
            let mut lanes = self.lanes.lock().unwrap();
            lanes.retain(|lane| !handed_off.contains(&lane.lane_id));
        }

        fn pending_exact_handoffs(&self) -> Vec<BrowserLaneId> {
            self.exact_handoffs.lock().unwrap().clone()
        }

        async fn close_all_for_test(&self) -> Result<CloseResult, BrowserPlatformError> {
            self.close_all_calls.fetch_add(1, Ordering::AcqRel);
            let mut lanes = self.lanes.lock().unwrap();
            let closed = lanes.len();
            lanes.clear();
            Ok(CloseResult {
                closed,
                already_closed: closed == 0,
                ..Default::default()
            })
        }
    }

    #[async_trait]
    impl BrowserLaneClientPort for FakeLaneClient {
        fn task_resource_key(&self) -> String {
            self.task_resource_key
                .get_or_init(|| {
                    let sequence = FAKE_TASK_RESOURCE_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
                    format!("fake-task-resource-{sequence}")
                })
                .clone()
        }

        fn handoff_bound_lane_cleanup(
            &self,
            lane_id: BrowserLaneId,
        ) -> Result<(), BrowserPlatformError> {
            self.exact_handoffs.lock().unwrap().push(lane_id);
            Ok(())
        }

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
            assert!(
                !Self::take_injected_panic(&self.panic_list_remaining),
                "injected fake Lane list panic"
            );
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
            assert!(
                !Self::take_injected_panic(&self.panic_close_remaining),
                "injected fake Lane close panic"
            );
            let active = self.close_active.fetch_add(1, Ordering::AcqRel) + 1;
            self.close_max_active.fetch_max(active, Ordering::AcqRel);
            if self.block_close.load(Ordering::Acquire) {
                let (released, wake) = &self.close_release;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
            }
            if self.fail_close.load(Ordering::Acquire) {
                self.close_active.fetch_sub(1, Ordering::AcqRel);
                self.close_called.notify_one();
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "The fake Lane close failed permanently.",
                    true,
                    "Escalate to owner cleanup.",
                )
                .for_lane(lane_id.clone()));
            }
            let mut lanes = self.lanes.lock().unwrap();
            let before = lanes.len();
            lanes.retain(|lane| &lane.lane_id != lane_id);
            let closed = usize::from(lanes.len() != before);
            self.close_active.fetch_sub(1, Ordering::AcqRel);
            self.close_called.notify_one();
            Ok(CloseResult {
                closed,
                already_closed: closed == 0,
                ..Default::default()
            })
        }

        async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError> {
            if self.reject_public_close_all.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::InvalidCallerIdentity,
                    "The fake public cleanup capability is stale.",
                    false,
                    "Use trusted task teardown authority.",
                ));
            }
            self.close_all_for_test().await
        }

        async fn apply_presentation_intent(
            &self,
            lane_id: &BrowserLaneId,
            intent: BrowserPresentationIntent,
        ) -> Result<(), BrowserPlatformError> {
            // A fake has no Chromium Host and therefore no window. Record the
            // report so a test can assert what the tool layer forwarded, and
            // accept it; the real resolution lives in the Hub.
            self.presentation_intents
                .lock()
                .unwrap()
                .push((lane_id.clone(), intent));
            Ok(())
        }
    }

    fn facade(client: Arc<FakeLaneClient>) -> ManagedBrowserFacade {
        ManagedBrowserFacade {
            client,
            workspace_dir: None,
        }
    }

    fn cleanup_test_dispatcher(
        max_authorities: usize,
        max_authorities_per_runtime: usize,
        workers: usize,
    ) -> CrawlCleanupDispatcher {
        CrawlCleanupDispatcher::new(CrawlCleanupDispatcherLimits {
            max_authorities,
            max_authorities_per_runtime,
            workers,
        })
        .expect("test cleanup dispatcher should start")
    }

    async fn wait_cleanup_dispatcher_idle(dispatcher: &CrawlCleanupDispatcher) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = dispatcher.snapshot();
                if snapshot.retained_authorities == 0
                    && snapshot.retained_intents == 0
                    && snapshot.active_workers == 0
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("crawl cleanup dispatcher should converge without another request");
    }

    fn queued_lane(client: &FakeLaneClient) -> BrowserLaneSnapshot {
        let mut lane = client.snapshot("default", BrowserIdentityMode::Primary);
        lane.lifecycle_state = LaneLifecycleState::Queued;
        lane.browser_epoch = 0;
        lane.queue = Some(nomifun_browser_platform::QueueMetadata {
            request_id: nomifun_browser_platform::QueueRequestId::new(),
            position: 1,
            recommended_concurrency: 1,
            owner_active: 0,
            owner_queued: 1,
            global_active: 0,
            global_queued: 1,
            retry_delay_ms: 1_000,
            reason_code: "system_memory_pressure".to_owned(),
        });
        lane
    }

    #[tokio::test]
    async fn queued_page_actions_are_errors_and_never_fake_dispatch() {
        let client = Arc::new(FakeLaneClient::default());
        let lane = queued_lane(&client);
        let lane_id = lane.lane_id.clone();
        client.lanes.lock().unwrap().push(lane);
        let facade = facade(Arc::clone(&client));

        for (action, input) in [
            (
                "navigate",
                json!({
                    "url": "https://example.test/",
                }),
            ),
            (
                "wait",
                json!({
                    "ms": 5_000,
                }),
            ),
        ] {
            let result = facade.execute(action, &input).await;
            assert!(result.is_error, "{action} must not report success: {}", result.content);
            let value: Value = serde_json::from_str(&result.content).unwrap();
            assert_eq!(value["ok"], false);
            assert_eq!(value["action"], action);
            assert_eq!(value["dispatched"], false);
            assert_eq!(value["lane"]["lane_id"], lane_id.as_str());
            assert_eq!(value["error"]["code"], "browser_capacity_queued");
            assert_eq!(
                value["error"]["metadata"]["reason_code"],
                "system_memory_pressure"
            );
            assert_eq!(value["retry_after_ms"], 1_000);
            assert_eq!(
                value["error"]["next_action"],
                value["next_action"],
                "top-level and structured retry guidance must remain identical"
            );
            assert!(
                value["next_action"]
                    .as_str()
                    .is_some_and(|next| next.contains("browser_status")),
                "{value}"
            );
        }

        assert!(
            client.operations.lock().unwrap().is_empty(),
            "queued navigate/wait must never reach the browser engine"
        );
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
            true,
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
            first_trusted_owner_field(&json!({"task_resource_key": "forged"})),
            Some("task_resource_key")
        );
        assert_eq!(
            first_trusted_owner_field(&json!({"runtime_cleanup_key": "forged"})),
            Some("runtime_cleanup_key")
        );
        assert_eq!(
            first_trusted_owner_field(&json!({"task_family_resource_key": "forged"})),
            Some("task_family_resource_key")
        );
        assert_eq!(
            first_trusted_owner_field(&json!({"task_resource_family_key": "forged"})),
            Some("task_resource_family_key")
        );
        assert_eq!(
            first_trusted_owner_field(&json!({"lane_id": "owner-scoped-handle"})),
            None
        );

        let sanitized = sanitize_operation_input(&json!({
            "url": "https://example.test/",
            "task_resource_key": "forged",
            "runtime_cleanup_key": "forged-runtime-cleanup",
            "task_family_resource_key": "forged-family",
            "task_resource_family_key": "forged-family-alias",
        }));
        assert_eq!(sanitized, json!({"url": "https://example.test/"}));
    }

    #[tokio::test]
    async fn managed_facade_rejects_every_shared_trusted_owner_field_without_dispatch() {
        let client = Arc::new(FakeLaneClient::default());
        let facade = facade(Arc::clone(&client));
        for field in TRUSTED_OWNER_INPUT_FIELDS {
            let result = facade
                .execute(
                    "navigate",
                    &json!({
                        "url": "https://example.test/",
                        (*field): "model-controlled",
                    }),
                )
                .await;

            assert!(result.is_error, "{field}: {}", result.content);
            assert!(
                result.content.contains("invalid_caller_identity"),
                "{field}: {}",
                result.content
            );
            assert!(result.content.contains(*field), "{field}: {}", result.content);
        }
        assert!(client.opens.lock().unwrap().is_empty());
        assert!(client.operations.lock().unwrap().is_empty());
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

        let oversized_url = format!(
            "https://example.test/{}",
            "界".repeat(MAX_CRAWL_URL_BYTES / 3)
        );
        assert!(crawl_urls(&json!({"urls": [oversized_url]})).is_err());
        assert!(
            crawl_urls(&json!({"urls": ["https://example.test/ok\nforged"]})).is_err()
        );

        let aggregate = (0..17)
            .map(|index| {
                let prefix = format!("https://example.test/{index}/");
                format!("{prefix}{}", "x".repeat(MAX_CRAWL_URL_BYTES - prefix.len()))
            })
            .collect::<Vec<_>>();
        assert!(crawl_urls(&json!({"urls": aggregate})).is_err());

        assert!(crawl_schema(&json!({"schema": {"title": "string"}})).is_ok());
        assert!(crawl_schema(&json!({"schema": "not-structured"})).is_err());
        assert!(
            crawl_schema(&json!({
                "schema": {"字段": "😀".repeat(
                    nomi_browser_engine::actions::MAX_EXTRACT_SCHEMA_BYTES
                )}
            }))
            .is_err()
        );
        let mut deep_schema = Value::Null;
        for _ in 0..nomi_browser_engine::actions::MAX_EXTRACT_SCHEMA_DEPTH {
            deep_schema = json!({"nested": deep_schema});
        }
        assert!(crawl_schema(&json!({"schema": deep_schema})).is_err());
    }

    #[test]
    fn crawl_batch_retained_budget_caps_many_unicode_results_and_serialization() {
        let urls = (0..MAX_CRAWL_URLS)
            .map(|index| format!("https://example.test/{index}"))
            .collect::<Vec<_>>();
        let request = ManagedCrawlRequest {
            urls: urls.clone(),
            requested_concurrency: MAX_CRAWL_CONCURRENCY,
            auto_concurrency: false,
            identity_mode: BrowserIdentityMode::Anonymous,
            schema: None,
            workspace_hint: None,
        };
        let budget = CrawlBatchRetainedBudget::new(urls.len());
        let results = urls
            .into_iter()
            .map(|url| {
                budget.retain(json!({
                    "url": url,
                    "ok": true,
                    "result": {"message": "😀".repeat(256 * 1024)},
                }))
            })
            .collect::<Vec<_>>();
        assert!(results.iter().any(|result| {
            result["error"]["code"] == "crawl_batch_byte_limit"
        }));

        let rendered = crawl_batch_result(&request, MAX_CRAWL_CONCURRENCY, results);
        assert!(!rendered.content.is_empty());
        assert!(rendered.content.len() <= MAX_CRAWL_BATCH_OUTPUT_BYTES);
        let parsed: Value = serde_json::from_str(&rendered.content).unwrap();
        assert_eq!(parsed["results"].as_array().unwrap().len(), MAX_CRAWL_URLS);
    }

    #[test]
    fn crawl_item_limit_replaces_one_oversized_value_with_compact_error() {
        let budget = CrawlBatchRetainedBudget::new(1);
        let retained = budget.retain(json!({
            "url": "https://example.test/huge",
            "ok": true,
            "result": "界".repeat(MAX_CRAWL_ITEM_RETAINED_BYTES / 3 + 1),
        }));
        assert_eq!(retained["error"]["code"], "crawl_item_byte_limit");
        assert!(
            nomi_browser_engine::actions::serialized_json_bytes_at_most(
                &retained,
                CRAWL_RESULT_BASE_RESERVE_BYTES,
            )
            .is_ok()
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
    async fn crawl_many_cancellation_during_open_does_not_publish_name_cleanup() {
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
        assert!(client.closes.lock().unwrap().is_empty());
        assert!(client.pending_exact_handoffs().is_empty());
        // This fake inserts before its await and has no Hub admission guard.
        // Production cleanup belongs to LaneStartWaiter/abandoned_lane_starts;
        // managed code must not rescan a reusable name as a second authority.
        client.lanes.lock().unwrap().clear();
    }

    #[tokio::test]
    async fn cancelled_crawl_storm_dedupes_cleanup_and_has_fixed_close_concurrency() {
        const LANE_COUNT: usize = 8;
        const CANCELLED_BATCHES: usize = 32;
        let client = Arc::new(FakeLaneClient::default());
        client.block_close.store(true, Ordering::Release);
        let lanes = (0..LANE_COUNT)
            .map(|index| {
                client.snapshot(
                    &format!("cancel-storm-{index}"),
                    BrowserIdentityMode::Anonymous,
                )
            })
            .collect::<Vec<_>>();
        client.lanes.lock().unwrap().extend(lanes.clone());
        let dispatcher = cleanup_test_dispatcher(LANE_COUNT, LANE_COUNT, 2);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let ready = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..CANCELLED_BATCHES {
            let dispatcher = dispatcher.clone();
            let client = Arc::clone(&client_port);
            let lanes = lanes.clone();
            let ready = Arc::clone(&ready);
            tasks.push(tokio::spawn(async move {
                let mut cleanup = CrawlBatchCleanup::new_with_dispatcher(client, dispatcher);
                for lane in &lanes {
                    cleanup.track_owned_lane(lane);
                }
                ready.fetch_add(1, Ordering::AcqRel);
                std::future::pending::<()>().await;
            }));
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while ready.load(Ordering::Acquire) != CANCELLED_BATCHES {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all cancelled batches should own their cleanup guards");
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            assert!(task.await.unwrap_err().is_cancelled());
        }

        let retained = dispatcher.snapshot();
        assert_eq!(retained.retained_authorities, LANE_COUNT);
        assert_eq!(retained.retained_intents, LANE_COUNT);
        assert!(retained.active_workers <= 2);
        assert!(retained.max_active_workers <= 2);

        client.release_all_closes();
        wait_cleanup_dispatcher_idle(&dispatcher).await;
        let completed = dispatcher.snapshot();
        assert_eq!(completed.completed_authorities, LANE_COUNT);
        assert_eq!(client.closes.lock().unwrap().len(), LANE_COUNT);
        assert!(client.lanes.lock().unwrap().is_empty());
        assert!(client.close_max_active.load(Ordering::Acquire) <= 2);
        dispatcher.shutdown_and_join();
    }

    #[tokio::test]
    async fn saturated_cleanup_dispatcher_hands_overflow_to_exact_hub_ledger() {
        const CAPACITY: usize = 4;
        const LANE_COUNT: usize = 12;
        let client = Arc::new(FakeLaneClient::default());
        client.block_close.store(true, Ordering::Release);
        let lanes = (0..LANE_COUNT)
            .map(|index| {
                client.snapshot(
                    &format!("capacity-{index}"),
                    BrowserIdentityMode::Anonymous,
                )
            })
            .collect::<Vec<_>>();
        client.lanes.lock().unwrap().extend(lanes.clone());
        let dispatcher = cleanup_test_dispatcher(CAPACITY, CAPACITY, 2);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let producers = lanes
            .into_iter()
            .map(|lane| {
                let dispatcher = dispatcher.clone();
                let client = Arc::clone(&client_port);
                std::thread::spawn(move || {
                    let mut cleanup =
                        CrawlBatchCleanup::new_with_dispatcher(client, dispatcher);
                    cleanup.track_owned_lane(&lane);
                    drop(cleanup);
                })
            })
            .collect::<Vec<_>>();

        for producer in producers {
            producer
                .join()
                .expect("bounded cleanup handoff must not wait for Lane close");
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = dispatcher.snapshot();
                assert!(snapshot.retained_authorities <= CAPACITY);
                if snapshot.retained_authorities == CAPACITY
                    && snapshot.retained_intents == CAPACITY
                    && snapshot.active_workers == 2
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("local exact dispatcher should stop at its hard authority bound");
        assert_eq!(
            client.pending_exact_handoffs().len(),
            LANE_COUNT - CAPACITY,
            "overflow must be handed to exact Hub Lane authority"
        );

        client.release_all_closes();
        wait_cleanup_dispatcher_idle(&dispatcher).await;
        let completed = dispatcher.snapshot();
        assert_eq!(completed.completed_authorities, CAPACITY);
        assert!(completed.max_active_workers <= 2);
        assert_eq!(client.closes.lock().unwrap().len(), CAPACITY);
        assert_eq!(client.close_all_calls.load(Ordering::Acquire), 0);
        client.apply_exact_handoffs();
        assert!(client.lanes.lock().unwrap().is_empty());
        assert!(client.close_max_active.load(Ordering::Acquire) <= 2);
        dispatcher.shutdown_and_join();
    }

    #[tokio::test]
    async fn delayed_old_exact_handoff_preserves_new_batch_observe_and_other_runtime() {
        let client = Arc::new(FakeLaneClient::default());
        client.fail_close.store(true, Ordering::Release);
        let old_lane = client.snapshot("old-crawl-batch", BrowserIdentityMode::Anonymous);
        client.lanes.lock().unwrap().push(old_lane.clone());
        let dispatcher = cleanup_test_dispatcher(4, 4, 1);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let mut cleanup =
            CrawlBatchCleanup::new_with_dispatcher(client_port, dispatcher.clone());
        cleanup.track_owned_lane(&old_lane);
        drop(cleanup);

        wait_cleanup_dispatcher_idle(&dispatcher).await;
        assert_eq!(
            client.closes.lock().unwrap().len(),
            CRAWL_CLEANUP_EXACT_RETRY_ESCALATE as usize
        );
        assert_eq!(client.pending_exact_handoffs(), vec![old_lane.lane_id.clone()]);

        let new_batch_lane =
            client.snapshot("new-crawl-batch", BrowserIdentityMode::Anonymous);
        let observe_lane = client.snapshot("later-observe", BrowserIdentityMode::Primary);
        let mut other_runtime_lane =
            client.snapshot("other-runtime-download", BrowserIdentityMode::Anonymous);
        other_runtime_lane.lane_key.runtime_instance_id = "other-runtime".to_owned();
        other_runtime_lane.caller.runtime_instance_id = "other-runtime".to_owned();
        other_runtime_lane.caller.owner_lease_id =
            nomifun_browser_platform::OwnerLeaseId("other-owner-lease".to_owned());
        client.lanes.lock().unwrap().extend([
            new_batch_lane.clone(),
            observe_lane.clone(),
            other_runtime_lane.clone(),
        ]);
        client.apply_exact_handoffs();
        let remaining = client
            .lanes
            .lock()
            .unwrap()
            .iter()
            .map(|lane| lane.lane_id.clone())
            .collect::<HashSet<_>>();
        assert_eq!(
            remaining,
            HashSet::from([
                new_batch_lane.lane_id,
                observe_lane.lane_id,
                other_runtime_lane.lane_id,
            ])
        );
        assert_eq!(client.close_all_calls.load(Ordering::Acquire), 0);
        let completed = dispatcher.snapshot();
        assert_eq!(completed.retained_authorities, 0);
        assert_eq!(completed.max_active_workers, 1);
        dispatcher.shutdown_and_join();
    }

    #[test]
    fn unavailable_dispatcher_hands_only_owned_lane_to_exact_hub_ledger() {
        let client = Arc::new(FakeLaneClient::default());
        let owned_lane = client.snapshot("dropped-crawl", BrowserIdentityMode::Anonymous);
        let survivor = client.snapshot("later-observe", BrowserIdentityMode::Primary);
        client
            .lanes
            .lock()
            .unwrap()
            .extend([owned_lane.clone(), survivor.clone()]);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let mut cleanup = CrawlBatchCleanup::new_with_unavailable_dispatcher(client_port);
        cleanup.track_owned_lane(&owned_lane);
        drop(cleanup);

        assert_eq!(
            client.pending_exact_handoffs(),
            vec![owned_lane.lane_id.clone()]
        );
        assert_eq!(client.close_all_calls.load(Ordering::Acquire), 0);
        client.apply_exact_handoffs();
        let remaining = client.lanes.lock().unwrap().clone();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].lane_id, survivor.lane_id);
    }

    #[test]
    fn last_worker_exit_hands_retained_exact_lane_to_hub() {
        let client = Arc::new(FakeLaneClient::default());
        let abandoned_lane =
            client.snapshot("worker-exit-crawl", BrowserIdentityMode::Anonymous);
        let survivor = client.snapshot("healthy-observe", BrowserIdentityMode::Primary);
        client
            .lanes
            .lock()
            .unwrap()
            .extend([abandoned_lane.clone(), survivor.clone()]);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let intent = CrawlCleanupIntent {
            task_resource_key: client_port.task_resource_key(),
            client: Arc::clone(&client_port),
            lane_name: abandoned_lane.lane_key.lane_name.clone(),
            lane_ids: HashSet::from([abandoned_lane.lane_id.clone()]),
            due_at: Instant::now(),
            attempts: 2,
            status: CrawlCleanupIntentStatus::InFlight,
        };
        let inner = Arc::new(CrawlCleanupDispatcherInner {
            limits: CrawlCleanupDispatcherLimits {
                max_authorities: 4,
                max_authorities_per_runtime: 4,
                workers: 1,
            },
            state: StdMutex::new(CrawlCleanupDispatcherState {
                next_intent_id: 1,
                intents: HashMap::from([(1, intent)]),
                active_workers: 1,
                max_active_workers: 1,
                completed_authorities: 0,
                shutdown: false,
            }),
            work_available: Condvar::new(),
            live_workers: AtomicUsize::new(1),
        });

        drop(CrawlCleanupWorkerLiveness {
            inner: Arc::clone(&inner),
        });

        assert_eq!(inner.live_workers.load(Ordering::Acquire), 0);
        let state = inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(state.intents.is_empty());
        assert_eq!(state.active_workers, 0);
        drop(state);
        assert_eq!(
            client.pending_exact_handoffs(),
            vec![abandoned_lane.lane_id]
        );
        assert_eq!(client.close_all_calls.load(Ordering::Acquire), 0);
        client.apply_exact_handoffs();
        let remaining = client.lanes.lock().unwrap().clone();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].lane_id, survivor.lane_id);
    }

    #[tokio::test]
    async fn panicking_exact_close_hands_only_that_lane_to_hub() {
        const WORKERS: usize = 2;
        let close_panic_client = Arc::new(FakeLaneClient::default());
        close_panic_client
            .panic_close_remaining
            .store(CRAWL_CLEANUP_EXACT_RETRY_ESCALATE as usize, Ordering::Release);
        let close_panic_lane = close_panic_client.snapshot(
            "panic-close",
            BrowserIdentityMode::Anonymous,
        );
        close_panic_client
            .lanes
            .lock()
            .unwrap()
            .push(close_panic_lane.clone());

        let healthy_client = Arc::new(FakeLaneClient::default());
        let healthy_lane = healthy_client.snapshot("healthy-after-panic", BrowserIdentityMode::Anonymous);
        healthy_client
            .lanes
            .lock()
            .unwrap()
            .push(healthy_lane.clone());

        let dispatcher = cleanup_test_dispatcher(16, 8, WORKERS);
        let close_panic_port: Arc<dyn BrowserLaneClientPort> = close_panic_client.clone();
        let mut close_cleanup =
            CrawlBatchCleanup::new_with_dispatcher(close_panic_port, dispatcher.clone());
        close_cleanup.track_owned_lane(&close_panic_lane);
        drop(close_cleanup);

        let healthy_port: Arc<dyn BrowserLaneClientPort> = healthy_client.clone();
        let mut healthy_cleanup =
            CrawlBatchCleanup::new_with_dispatcher(healthy_port, dispatcher.clone());
        healthy_cleanup.track_owned_lane(&healthy_lane);
        drop(healthy_cleanup);

        wait_cleanup_dispatcher_idle(&dispatcher).await;
        let completed = dispatcher.snapshot();
        assert_eq!(completed.live_workers, WORKERS);
        assert_eq!(completed.retained_authorities, 0);
        assert!(completed.max_active_workers <= WORKERS);
        assert_eq!(
            close_panic_client.pending_exact_handoffs(),
            vec![close_panic_lane.lane_id.clone()]
        );
        close_panic_client.apply_exact_handoffs();
        assert!(close_panic_client.lanes.lock().unwrap().is_empty());
        assert!(healthy_client.lanes.lock().unwrap().is_empty());
        assert_eq!(close_panic_client.close_all_calls.load(Ordering::Acquire), 0);
        assert_eq!(healthy_client.closes.lock().unwrap().len(), 1);
        dispatcher.shutdown_and_join();
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn normal_batch_cleanup_has_fixed_close_concurrency() {
        const LANE_COUNT: usize = 12;
        let client = Arc::new(FakeLaneClient::default());
        client.block_close.store(true, Ordering::Release);
        let lanes = (0..LANE_COUNT)
            .map(|index| {
                client.snapshot(
                    &format!("normal-close-{index}"),
                    BrowserIdentityMode::Anonymous,
                )
            })
            .collect::<Vec<_>>();
        let lane_ids = lanes
            .iter()
            .map(|lane| lane.lane_id.clone())
            .collect::<Vec<_>>();
        client.lanes.lock().unwrap().extend(lanes);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let cleanup = tokio::spawn(async move {
            close_lane_ids_until(
                client_port,
                lane_ids,
                tokio::time::Instant::now() + Duration::from_secs(3),
            )
            .await
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            while client.close_active.load(Ordering::Acquire)
                != CRAWL_CLEANUP_CLOSE_CONCURRENCY
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("normal cleanup should fill only its fixed worker window");
        assert_eq!(
            client.close_max_active.load(Ordering::Acquire),
            CRAWL_CLEANUP_CLOSE_CONCURRENCY
        );

        client.release_all_closes();
        let results = cleanup.await.expect("normal cleanup task joins");
        assert_eq!(results.len(), LANE_COUNT);
        assert!(results.values().all(Result::is_ok));
        assert_eq!(client.closes.lock().unwrap().len(), LANE_COUNT);
        assert!(client.lanes.lock().unwrap().is_empty());
        assert_eq!(
            client.close_max_active.load(Ordering::Acquire),
            CRAWL_CLEANUP_CLOSE_CONCURRENCY
        );
    }

    #[tokio::test]
    async fn cleanup_drop_outside_tokio_closes_exact_lane_automatically() {
        let client = Arc::new(FakeLaneClient::default());
        let lane = client.snapshot("outside-runtime", BrowserIdentityMode::Anonymous);
        client.lanes.lock().unwrap().push(lane.clone());
        let dispatcher = cleanup_test_dispatcher(4, 4, 1);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let producer_dispatcher = dispatcher.clone();
        std::thread::spawn(move || {
            assert!(
                tokio::runtime::Handle::try_current().is_err(),
                "fixture must exercise Drop without a caller Tokio runtime"
            );
            let mut cleanup =
                CrawlBatchCleanup::new_with_dispatcher(client_port, producer_dispatcher);
            cleanup.track_owned_lane(&lane);
            drop(cleanup);
        })
        .join()
        .expect("outside-runtime producer joins");

        wait_cleanup_dispatcher_idle(&dispatcher).await;
        let completed = dispatcher.snapshot();
        assert_eq!(completed.completed_authorities, 1);
        assert_eq!(client.closes.lock().unwrap().len(), 1);
        assert!(client.lanes.lock().unwrap().is_empty());
        assert!(completed.max_active_workers <= 1);
        dispatcher.shutdown_and_join();
    }

    #[tokio::test]
    async fn empty_guard_from_cancelled_open_publishes_no_cleanup() {
        let client = Arc::new(FakeLaneClient::default());
        let dispatcher = cleanup_test_dispatcher(4, 4, 1);
        let client_port: Arc<dyn BrowserLaneClientPort> = client.clone();
        let cleanup = CrawlBatchCleanup::new_with_dispatcher(client_port, dispatcher.clone());
        drop(cleanup);

        assert_eq!(dispatcher.snapshot().retained_authorities, 0);
        assert!(client.pending_exact_handoffs().is_empty());
        assert_eq!(client.close_all_calls.load(Ordering::Acquire), 0);
        assert_eq!(dispatcher.snapshot().completed_authorities, 0);
        dispatcher.shutdown_and_join();
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

    #[test]
    fn presentation_intent_accepts_intent_and_rejects_a_mechanism() {
        assert_eq!(
            parse_presentation_intent(&json!({})).unwrap(),
            BrowserPresentationIntent::Unattended,
            "routine work is the default"
        );
        assert_eq!(
            parse_presentation_intent(&json!({"presentation": "attended"})).unwrap(),
            BrowserPresentationIntent::Attended
        );
        assert_eq!(
            parse_presentation_intent(&json!({"presentation": " Attended "})).unwrap(),
            BrowserPresentationIntent::Attended,
            "case and padding must not change the meaning"
        );

        // The model states intent, never a mechanism. Guessing the wrong
        // vocabulary must be reported, not silently downgraded to routine.
        for mechanism in ["headless", "headful", "external", "visible", "foreground"] {
            let error = parse_presentation_intent(&json!({"presentation": mechanism}))
                .expect_err(mechanism);
            assert!(
                error.contains("does not accept"),
                "{mechanism} should be rejected as a mechanism: {error}"
            );
        }
        assert!(parse_presentation_intent(&json!({"presentation": "wat"})).is_err());
        assert!(parse_presentation_intent(&json!({"presentation": true})).is_err());
    }

    #[tokio::test]
    async fn attended_open_forwards_the_intent_and_routine_open_does_not() {
        let client = Arc::new(FakeLaneClient::default());
        let sequence = AtomicU64::new(0);
        open_lane(
            client.as_ref(),
            None,
            &sequence,
            &json!({"lane_name": "attended", "presentation": "attended"}),
            false,
            "Opening a browser Lane failed",
        )
        .await;
        assert_eq!(
            client
                .presentation_intents
                .lock()
                .unwrap()
                .iter()
                .map(|(_, intent)| *intent)
                .collect::<Vec<_>>(),
            vec![BrowserPresentationIntent::Attended]
        );

        // Routine work must not spend a report at all: the Hub's escalation
        // allowance is small and each one replaces the Chromium Host.
        client.presentation_intents.lock().unwrap().clear();
        open_lane(
            client.as_ref(),
            None,
            &sequence,
            &json!({"lane_name": "routine"}),
            false,
            "Opening a browser Lane failed",
        )
        .await;
        assert!(client.presentation_intents.lock().unwrap().is_empty());
    }

    /// `presentation` is host policy consumed before dispatch; it must not reach
    /// the driver as if it were an action parameter.
    #[tokio::test]
    async fn presentation_is_stripped_from_the_dispatched_operation() {
        let client = Arc::new(FakeLaneClient::default());
        let lane = client.snapshot("attended-exec", BrowserIdentityMode::Primary);
        client.lanes.lock().unwrap().push(lane.clone());

        let result = execute_existing_operation(
            client.as_ref(),
            None,
            "observe",
            &json!({"lane_id": lane.lane_id.as_str(), "presentation": "attended"}),
            false,
        )
        .await;
        assert!(!result.is_error, "{result:?}");

        let operations = client.operations.lock().unwrap();
        assert_eq!(operations.len(), 1);
        assert!(
            operations[0].1.input.get("presentation").is_none(),
            "presentation must be sanitized out of the driver operation"
        );
        assert_eq!(
            client.presentation_intents.lock().unwrap().len(),
            1,
            "the intent must still have been reported to the host"
        );
    }

    /// A rejected `presentation` must stop the call rather than silently run the
    /// action with the wrong visibility assumption.
    #[tokio::test]
    async fn invalid_presentation_rejects_the_operation_without_dispatching() {
        let client = Arc::new(FakeLaneClient::default());
        let lane = client.snapshot("bad-presentation", BrowserIdentityMode::Primary);
        client.lanes.lock().unwrap().push(lane.clone());

        let result = execute_existing_operation(
            client.as_ref(),
            None,
            "observe",
            &json!({"lane_id": lane.lane_id.as_str(), "presentation": "headful"}),
            false,
        )
        .await;

        assert!(result.is_error);
        assert!(client.operations.lock().unwrap().is_empty());
        assert!(client.presentation_intents.lock().unwrap().is_empty());
    }
}
