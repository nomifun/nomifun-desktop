//! [`CdpBackend`]：方案 A 的 P0 浏览器后端。
//!
//! **铁律（spike 锁定）**：用 [`crate::transport::Connection`] 发**裸 CDP 命令**，
//! **绝不**碰 chromiumoxide 高层 `Browser`/`Page`。CDP 命令的**生成类型**（params/returns）
//! 经 `chromiumoxide::cdp::*` re-export 复用——那只是 serde 结构体，不是高层 API。
//!
//! 持有物（缺一不可）：
//! - [`Connection`]：已 connect + 起 attach loop + enable_auto_attach 的传输。
//! - `page_session`：一个 page target 的 sessionId（经 createTarget + attachedToTarget 取到），
//!   后续 navigate/screenshot 都发到它。
//! - `child`：托管的 chrome 进程句柄——**保活**，Drop 即清理整棵进程树（Builder 的
//!   kill_on_drop + 三平台清理网）。绝不能提前 drop，否则 chrome 残留。
//! - `_attach_loop`：attach 处理循环的 JoinHandle，保活让子 session 持续被登记。
//! - capabilities 快照（headful/display）。
//!
//! 错误映射（Task B 故意把 `TransportError` 与 `BrowserError` 解耦，留在此处映射）：
//! 见 [`map_transport_err`]。绝不 panic。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::browser::{
    CancelDownloadParams, EventDownloadProgress, EventDownloadWillBegin, SetDownloadBehaviorBehavior,
    SetDownloadBehaviorParams,
};
use chromiumoxide::cdp::browser_protocol::dom::{
    EnableParams as DomEnableParams, GetFrameOwnerParams, ResolveNodeParams,
};
use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EnableParams as FetchEnableParams, EventRequestPaused, FailRequestParams,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams as NetworkEnableParams, ErrorReason as NetworkErrorReason, ResourceType,
};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, CaptureScreenshotReturns,
    EnableParams as PageEnableParams, NavigateParams, NavigateReturns, PrintToPdfParams,
    PrintToPdfReturns,
};
use chromiumoxide::cdp::browser_protocol::storage::{
    GetCookiesParams as StorageGetCookiesParams, SetCookiesParams as StorageSetCookiesParams,
};
use chromiumoxide::cdp::browser_protocol::target::{
    CreateTargetParams, EventAttachedToTarget, EventDetachedFromTarget, EventTargetCrashed,
    EventTargetDestroyed, GetTargetsParams,
};
use chromiumoxide::cdp::js_protocol::runtime::{
    CallArgument, CallFunctionOnParams, EvaluateParams, ExecutionContextId, RemoteObjectId,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::aria_ref::{
    frame_prefix, RefRecord, RefTable, RefTableCapacityError, MAX_REFS_PER_GENERATION,
};
use crate::actions::{ActResult, Effect};
use crate::engine::{
    BrowserEngine, BrowserError, BrowserTabInfo, Capabilities, ElementEntry, LoadState, NavResult,
    Observation, ObserveOpts,
};
use crate::injected::{AbortOnDropTask, InjectError, InjectionManager};
use crate::launch::{
    BrowserHostLaunchMode, LaunchConfig, Launched,
    launch_chrome_with_cleanup_profile,
    terminate_launched_process_tree_and_cleanup_profile,
};
use crate::nav::{
    self, InflightCounter, LifecycleSignal, NavSettleState, NETWORK_IDLE_CAP, NETWORK_IDLE_QUIET,
    SETTLE_QUIET, SPA_SETTLE_TIMEOUT,
};
use crate::observe::{
    ensure_observation_bytes, serialized_json_bytes_bounded, FrameSnapshot,
    ObservationCapacityError,
    MAX_OBSERVATION_RETAINED_BYTES,
};
use crate::progress::{AbortReason, Progress};
use crate::redact;
use crate::session::{ReliableEventTaskBudget, TaskSessionAdmission};
use crate::tabs::{OopifEntry, TabHandles, TabRecord};
use crate::transport::{Connection, TransportError, ROOT_SESSION};
use crate::host::{
    HostCleanupLease, LaneOperationGate, TaskDownloadReservation,
    TaskDownloadReservationAuthority, TaskTabReservation, TaskTabReservationAuthority,
};
use crate::{EngineConfig, LaneEngineConfig, LaneId, TargetOwnership, TargetRoute};

/// 拿到新 page 的 `attachedToTarget` 事件的上限（flatten auto-attach 通常 <1s）。
const PAGE_ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
/// Must exceed transport::DEFAULT_COMMAND_TIMEOUT (30s). An unknown-id
/// cancellation may not use inventory absence until the create command future
/// has reached its own terminal response/timeout.
const PENDING_PAGE_CREATE_RECOVERY_TIMEOUT: Duration = Duration::from_secs(40);
/// Cleanup pressure is deliberately bounded. One cleanup is active and at
/// most this many wait behind it. Saturation poisons the Host and escalates to
/// whole-process cleanup, which is the only safe way to subsume an additional
/// target without retaining another unbounded cleanup intent.
const TARGET_CLEANUP_QUEUE_CAPACITY: usize = 32;
/// A single permanently failing close/finalizer must not keep a Host and its
/// sole worker alive forever. Exhaustion escalates to whole-Host teardown.
const TARGET_CLEANUP_JOB_BUDGET: Duration = Duration::from_secs(5);
/// Whole-process cleanup is attempted synchronously only for a bounded period;
/// on failure its exact authority is transferred to the launch module's
/// durable process relay/startup quarantine.
const TARGET_PROCESS_CLEANUP_ATTEMPTS: usize = 3;
const TARGET_PROCESS_CLEANUP_ATTEMPT_BUDGET: Duration = Duration::from_secs(5);
/// Firewall request classification (including DNS SSRF checks) runs only on
/// these Host-owned workers. The count is fixed for the Host lifetime.
const FIREWALL_REQUEST_WORKERS: usize = 4;
/// Reliable CDP delivery is intentionally lossless, so the consumer must place
/// an explicit bound immediately after receipt. Saturation fails closed instead
/// of accumulating events forever.
const FIREWALL_REQUEST_QUEUE_CAPACITY: usize = 128;
/// Slow human approvals are isolated from request classification so they do
/// not stall ordinary allow/block decisions.
const FIREWALL_APPROVAL_WORKERS: usize = 4;
const FIREWALL_APPROVAL_QUEUE_CAPACITY: usize = 32;
/// Explicit Host shutdown cancels the worker tree and joins it for this bounded
/// interval before using abort as the final fallback.
const FIREWALL_SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);
/// Saturation rejection itself cannot become another unbounded CDP wait.
const FIREWALL_OVERFLOW_REJECT_TIMEOUT: Duration = Duration::from_secs(1);
/// An unclaimed top-level target gets a brief nonce/create correlation window.
/// After that it is not useful browser state: it is closed through the same
/// bounded executor as every other abandoned target.
const QUARANTINED_TARGET_GRACE: Duration = Duration::from_millis(750);
const ROUTER_STATE_SWEEP_INTERVAL: Duration = Duration::from_millis(100);
/// Loss tombstones only bridge attach/arm event reordering. Keeping them for a
/// whole Host epoch makes normal tab churn an unbounded metadata leak.
const LOST_TARGET_TOMBSTONE_GRACE: Duration = Duration::from_secs(2);
/// These are transient, Host-global exception sets rather than useful live
/// task state. Saturation fails closed and advances to authoritative Host
/// teardown instead of retaining more target ids.
const MAX_QUARANTINED_TARGETS: usize = TARGET_CLEANUP_QUEUE_CAPACITY;
const MAX_ROUTER_CLEANUP_INFLIGHT: usize = TARGET_CLEANUP_QUEUE_CAPACITY + 1;
const MAX_PENDING_CREATE_INTENTS: usize = TARGET_CLEANUP_QUEUE_CAPACITY;
/// A single Lane should not accumulate an unbounded renderer set through
/// repeated `new_tab` actions or page-created popups. Eight keeps normal
/// multi-tab research usable while bounding the dominant per-Lane memory fanout.
const MAX_TABS_PER_LANE: usize = 8;
const MAX_TRACKED_TARGETS_PER_LANE: usize =
    MAX_TABS_PER_LANE + MAX_ROUTER_CLEANUP_INFLIGHT;
/// A retired Lane cleanup inventory is read from a Host-global
/// `Target.getTargets` response.  The transport already owns the decoded JSON;
/// cleanup must not clone a second, nearly transport-sized copy before it can
/// apply the per-Lane target bound.
const MAX_TARGET_INVENTORY_ENTRIES_PER_HOST: usize = 4_096;
const MAX_TARGET_INVENTORY_STRING_BYTES: usize = 4 * 1024 * 1024;
const MAX_LANE_TARGET_LINEAGE_STRING_BYTES: usize =
    MAX_TRACKED_TARGETS_PER_LANE * crate::session::MAX_CDP_IDENTIFIER_BYTES;
/// Debug timing is optional metadata.  Bound both cardinality and owned key
/// bytes independently so 2,000 protocol-valid 4 KiB request ids cannot retain
/// an otherwise surprising multi-megabyte side table per tab.
const MAX_DEBUG_REQUEST_TIMESTAMPS: usize = 2_000;
const MAX_DEBUG_REQUEST_TIMESTAMP_KEY_BYTES: usize = crate::session::MAX_CDP_IDENTIFIER_BYTES;
const MAX_DEBUG_REQUEST_TIMESTAMP_TOTAL_KEY_BYTES: usize = 256 * 1024;
/// Per-tab OOPIF tracking is deliberately finite.  A pathological frame tree
/// must not create an unbounded number of injection tasks even while the Host
/// itself remains healthy for sibling Lanes.
const MAX_OOPIFS_PER_TAB: usize = 32;
/// A Host may route only this many downloads at once.  Admission above this
/// bound is denied and cancelled at CDP rather than retaining more state.
const MAX_PENDING_DOWNLOADS_PER_HOST: usize = 64;
const MAX_QUARANTINED_DOWNLOADS_PER_HOST: usize = 64;
/// Downloads that never publish a terminal progress event are cancelled and
/// forgotten after this deadline; their exact Host-staging artifacts are also
/// removed.
const DOWNLOAD_ROUTE_TTL: Duration = Duration::from_secs(5 * 60);
/// A cancel acknowledgement is not proof that Chromium stopped writing. If no
/// terminal event arrives within this grace, the Host is poisoned and exact
/// process shutdown becomes the only release proof for retained reservations.
const DOWNLOAD_CANCEL_TERMINAL_GRACE: Duration = Duration::from_secs(5);
const DOWNLOAD_RECONCILE_INTERVAL: Duration = Duration::from_secs(15);
/// Every admitted or cancellation-quarantined GUID can own three deterministic
/// staging names (`GUID`, `.crdownload`, and `.tmp`). Keep enough exact retry
/// slots for the full 64 active + 64 rejected inventory at once. If historical
/// cleanup debt ever fills this table, authority is promoted to the dedicated
/// staging directory and the Host is poisoned before accepting more work.
const DOWNLOAD_STAGING_PATHS_PER_GUID: usize = 3;
const MAX_DOWNLOAD_CLEANUP_RETRIES: usize =
    (MAX_PENDING_DOWNLOADS_PER_HOST + MAX_QUARANTINED_DOWNLOADS_PER_HOST)
        * DOWNLOAD_STAGING_PATHS_PER_GUID;
const MAX_DOWNLOAD_STAGING_SCAN_ENTRIES: usize = 512;
const MAX_DOWNLOAD_SUGGESTED_FILENAME_BYTES: usize = 1024;

const fn tab_capacity_available(current_tabs: usize) -> bool {
    current_tabs < MAX_TABS_PER_LANE
}

const fn oopif_capacity_available(current_oopifs: usize) -> bool {
    current_oopifs < MAX_OOPIFS_PER_TAB
}

const fn download_capacity_available(current_downloads: usize) -> bool {
    current_downloads < MAX_PENDING_DOWNLOADS_PER_HOST
}

/// observe 时等主帧 utility-world context 物化的上限（fresh navigate 后 world 创建有延迟；
/// 通常 <500ms）。超时 → `NavFailed{kind:"context"}`（调用方可短重试）。
const OBSERVE_CONTEXT_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for enqueueing and acknowledging a Host router ordering fence.
/// A wedged handler or a permanently busy mailbox must surface a retryable
/// cleanup failure instead of hanging Lane shutdown forever.
const HOST_ROUTER_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);

/// 把传输/会话层错误映射到引擎错误。**绝不 panic**；让模型读到可路由的语义。
///
/// - `Timeout` → `NavFailed`（多见于 navigate/load 等不来；语义是「这次操作没完成」）。
/// - `Closed` → `SessionLost{recoverable:false}`（整个 Host/CDP 连接没了）。
/// - `SessionClosed` → `TargetClosed`（仅该 target/page 已关闭）。
/// - `SessionCrashed` → `TargetCrashed`（仅该 target 崩溃，可选其它 tab 或新建 target）。
/// - `Cdp{code,message}` → `Other`（浏览器侧拒绝；带上 code/message 供诊断）。
/// - `Protocol` → `Other`（我方序列化/路由不变量问题）。
pub fn map_transport_err(e: TransportError) -> BrowserError {
    match e {
        TransportError::Timeout => BrowserError::NavFailed {
            kind: "cdp command timed out".into(),
        },
        TransportError::Closed => BrowserError::SessionLost { recoverable: false },
        TransportError::SessionClosed => BrowserError::TargetClosed,
        TransportError::SessionCrashed => BrowserError::TargetCrashed,
        TransportError::Cdp { code, message } => {
            BrowserError::Other(format!("cdp error {code}: {message}"))
        }
        TransportError::Protocol(msg) => BrowserError::Other(format!("cdp protocol error: {msg}")),
    }
}

/// 把**注入管线**错误 [`InjectError`] 映射到引擎错误。**绝不 panic**；穷尽 match（不写 `_`，
/// 这样 injected.rs 新增变体时编译期就逼我们补语义，而非静默归入 `Other`）。observe（Task 6）
/// 调 `call_injected` 拿 aria 时用它。
///
/// - `Transport` → 复用 [`map_transport_err`]（底层传输/会话语义不在这里重新分类）。
/// - `ContextNotReady` → `NavFailed{kind:"context"}`（utility world 还没物化/正在导航，语义近
///   「这次没拿到可用上下文」，调用方可短重试后再报）。
/// - `JsException` / `Protocol` → `Other`（页面侧 JS 抛异常 / CDP 回包形状异常——都带原文供诊断）。
// observe（Task 6）在 call_injected / 帧路由边界把 InjectError 翻成 BrowserError 时调它。
pub(crate) fn map_inject_err(e: InjectError) -> BrowserError {
    match e {
        InjectError::Transport(t) => map_transport_err(t),
        InjectError::ContextNotReady { .. } => BrowserError::NavFailed {
            kind: "context".into(),
        },
        InjectError::ContextCapacityExceeded { limit } => BrowserError::Blocked {
            reason: format!(
                "utility-world context limit exceeded ({limit}); reload or simplify the frame tree"
            ),
        },
        InjectError::RefCapacityExceeded {
            limit,
            current,
            required,
        } => BrowserError::Blocked {
            reason: format!(
                "observe would retain too many element refs for one task generation \
                 (limit={limit}, attempted_total={current}, frame_refs={required}). The partial \
                 generation was discarded; simplify the page or reduce observe depth, then run \
                 a fresh observe"
            ),
        },
        InjectError::ObservationCapacityExceeded {
            limit,
            current,
            frame_bytes,
        } => BrowserError::Blocked {
            reason: format!(
                "observe exceeded the per-task snapshot byte limit \
                 (limit={limit}, attempted_total={current}, frame_bytes={frame_bytes}). The \
                 partial generation was discarded; simplify the page or reduce observe depth, \
                 then run a fresh observe"
            ),
        },
        InjectError::JsException(m) => BrowserError::Other(m),
        InjectError::Protocol(m) => BrowserError::Other(m),
    }
}

fn map_observation_capacity_err(error: ObservationCapacityError) -> BrowserError {
    BrowserError::Blocked {
        reason: format!(
            "observe exceeded the per-task retained byte limit (limit={}, attempted={}). The \
             partial generation was discarded; simplify the page or reduce observe depth, then \
             run a fresh observe",
            error.limit, error.attempted
        ),
    }
}

#[derive(Clone, Debug)]
struct ValidatedTargetInfo {
    target_id: String,
}

/// Parse the root target inventory used by cleanup proofs.
///
/// Cleanup must fail closed: a malformed entry cannot be skipped because
/// doing so could turn an unknown live target into a false absence proof.
fn validated_target_inventory(
    result: &serde_json::Value,
) -> Result<Vec<ValidatedTargetInfo>, &'static str> {
    let target_infos = result
        .get("targetInfos")
        .and_then(serde_json::Value::as_array)
        .ok_or("Target.getTargets response is missing targetInfos")?;
    target_infos
        .iter()
        .map(|target_info| {
            let target_id = target_info
                .get("targetId")
                .and_then(serde_json::Value::as_str)
                .ok_or("Target.getTargets targetInfo is missing a string targetId")?;
            let target_type = target_info
                .get("type")
                .and_then(serde_json::Value::as_str)
                .ok_or("Target.getTargets targetInfo is missing a string type")?;
            let opener_id = match target_info.get("openerId") {
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or("Target.getTargets targetInfo has a non-string openerId")?,
                ),
                None => None,
            };
            let _ = (target_type, opener_id);
            Ok(ValidatedTargetInfo {
                target_id: target_id.to_string(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedLaneTargetInventoryError {
    Malformed(&'static str),
    HostEntryLimit,
    HostStringByteLimit,
    IdentifierByteLimit,
    LaneTargetLimit,
    LaneStringByteLimit,
}

impl std::fmt::Display for BoundedLaneTargetInventoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => formatter.write_str(message),
            Self::HostEntryLimit => write!(
                formatter,
                "Target.getTargets inventory exceeds the Host entry limit ({MAX_TARGET_INVENTORY_ENTRIES_PER_HOST})"
            ),
            Self::HostStringByteLimit => write!(
                formatter,
                "Target.getTargets inventory exceeds the retained string byte limit ({MAX_TARGET_INVENTORY_STRING_BYTES})"
            ),
            Self::IdentifierByteLimit => write!(
                formatter,
                "Target.getTargets inventory contains an oversized CDP identifier"
            ),
            Self::LaneTargetLimit => write!(
                formatter,
                "Lane target lineage exceeds the target limit ({MAX_TRACKED_TARGETS_PER_LANE})"
            ),
            Self::LaneStringByteLimit => write!(
                formatter,
                "Lane target lineage exceeds the retained identifier byte limit ({MAX_LANE_TARGET_LINEAGE_STRING_BYTES})"
            ),
        }
    }
}

fn accumulate_inventory_string_bytes(
    value: &serde_json::Value,
    retained: &mut usize,
) -> Result<(), BoundedLaneTargetInventoryError> {
    match value {
        serde_json::Value::String(text) => add_inventory_string_bytes(retained, text.len()),
        serde_json::Value::Array(values) => {
            for value in values {
                accumulate_inventory_string_bytes(value, retained)?;
            }
            Ok(())
        }
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                add_inventory_string_bytes(retained, key.len())?;
                accumulate_inventory_string_bytes(value, retained)?;
            }
            Ok(())
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Ok(())
        }
    }
}

fn add_inventory_string_bytes(
    retained: &mut usize,
    bytes: usize,
) -> Result<(), BoundedLaneTargetInventoryError> {
    *retained = retained
        .checked_add(bytes)
        .ok_or(BoundedLaneTargetInventoryError::HostStringByteLimit)?;
    if *retained > MAX_TARGET_INVENTORY_STRING_BYTES {
        return Err(BoundedLaneTargetInventoryError::HostStringByteLimit);
    }
    Ok(())
}

fn bounded_lane_seed_targets<'a>(
    targets: impl IntoIterator<Item = &'a str>,
) -> Result<HashSet<String>, BoundedLaneTargetInventoryError> {
    let mut retained_bytes = 0usize;
    let mut bounded = HashSet::new();
    for target_id in targets {
        if target_id.len() > crate::session::MAX_CDP_IDENTIFIER_BYTES {
            return Err(BoundedLaneTargetInventoryError::IdentifierByteLimit);
        }
        if bounded.contains(target_id) {
            continue;
        }
        if bounded.len() >= MAX_TRACKED_TARGETS_PER_LANE {
            return Err(BoundedLaneTargetInventoryError::LaneTargetLimit);
        }
        retained_bytes = retained_bytes
            .checked_add(target_id.len())
            .ok_or(BoundedLaneTargetInventoryError::LaneStringByteLimit)?;
        if retained_bytes > MAX_LANE_TARGET_LINEAGE_STRING_BYTES {
            return Err(BoundedLaneTargetInventoryError::LaneStringByteLimit);
        }
        bounded.insert(target_id.to_owned());
    }
    Ok(bounded)
}

fn validated_target_info_ref(
    target_info: &serde_json::Value,
) -> Result<(&str, &str, Option<&str>), BoundedLaneTargetInventoryError> {
    let target_id = target_info
        .get("targetId")
        .and_then(serde_json::Value::as_str)
        .ok_or(BoundedLaneTargetInventoryError::Malformed(
            "Target.getTargets targetInfo is missing a string targetId",
        ))?;
    let target_type = target_info
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(BoundedLaneTargetInventoryError::Malformed(
            "Target.getTargets targetInfo is missing a string type",
        ))?;
    let opener_id = match target_info.get("openerId") {
        Some(value) => Some(value.as_str().ok_or(
            BoundedLaneTargetInventoryError::Malformed(
                "Target.getTargets targetInfo has a non-string openerId",
            ),
        )?),
        None => None,
    };
    if target_id.len() > crate::session::MAX_CDP_IDENTIFIER_BYTES
        || target_type.len() > crate::session::MAX_CDP_IDENTIFIER_BYTES
        || opener_id.is_some_and(|value| value.len() > crate::session::MAX_CDP_IDENTIFIER_BYTES)
    {
        return Err(BoundedLaneTargetInventoryError::IdentifierByteLimit);
    }
    Ok((target_id, target_type, opener_id))
}

/// Resolve one Lane's live target lineage without cloning the Host-global
/// target inventory. All inventory and per-Lane limits are checked before the
/// first inventory identifier is copied; the final vector takes ownership from
/// the lineage set instead of creating a second copy.
fn bounded_live_lane_targets(
    result: &serde_json::Value,
    mut lineage: HashSet<String>,
) -> Result<Vec<String>, BoundedLaneTargetInventoryError> {
    let target_infos = result
        .get("targetInfos")
        .and_then(serde_json::Value::as_array)
        .ok_or(BoundedLaneTargetInventoryError::Malformed(
            "Target.getTargets response is missing targetInfos",
        ))?;
    if target_infos.len() > MAX_TARGET_INVENTORY_ENTRIES_PER_HOST {
        return Err(BoundedLaneTargetInventoryError::HostEntryLimit);
    }
    let mut inventory_string_bytes = 0usize;
    for target_info in target_infos {
        accumulate_inventory_string_bytes(target_info, &mut inventory_string_bytes)?;
        let _ = validated_target_info_ref(target_info)?;
    }

    let mut lineage_string_bytes = lineage.iter().try_fold(0usize, |total, target_id| {
        total
            .checked_add(target_id.len())
            .ok_or(BoundedLaneTargetInventoryError::LaneStringByteLimit)
    })?;
    if lineage.len() > MAX_TRACKED_TARGETS_PER_LANE {
        return Err(BoundedLaneTargetInventoryError::LaneTargetLimit);
    }
    if lineage_string_bytes > MAX_LANE_TARGET_LINEAGE_STRING_BYTES {
        return Err(BoundedLaneTargetInventoryError::LaneStringByteLimit);
    }

    loop {
        let mut changed = false;
        for target_info in target_infos {
            let (target_id, target_type, opener_id) = validated_target_info_ref(target_info)?;
            if target_type != "page" {
                continue;
            }
            let Some(opener_id) = opener_id else {
                continue;
            };
            if !lineage.contains(opener_id) || lineage.contains(target_id) {
                continue;
            }
            if lineage.len() >= MAX_TRACKED_TARGETS_PER_LANE {
                return Err(BoundedLaneTargetInventoryError::LaneTargetLimit);
            }
            let Some(next_bytes) = lineage_string_bytes.checked_add(target_id.len()) else {
                return Err(BoundedLaneTargetInventoryError::LaneStringByteLimit);
            };
            if next_bytes > MAX_LANE_TARGET_LINEAGE_STRING_BYTES {
                return Err(BoundedLaneTargetInventoryError::LaneStringByteLimit);
            }
            lineage.insert(target_id.to_owned());
            lineage_string_bytes = next_bytes;
            changed = true;
        }
        if !changed {
            break;
        }
    }

    let mut live_targets = Vec::with_capacity(lineage.len());
    for target_info in target_infos {
        let (target_id, _, _) = validated_target_info_ref(target_info)?;
        if let Some(owned_target_id) = lineage.take(target_id) {
            live_targets.push(owned_target_id);
        }
    }
    Ok(live_targets)
}

async fn target_is_absent_from_browser(
    conn: &Connection,
    target_id: &str,
) -> Result<bool, TransportError> {
    let result = conn
        .send::<GetTargetsParams>(ROOT_SESSION, &GetTargetsParams::default())
        .await?;
    let target_infos = validated_target_inventory(&result)
        .map_err(|message| TransportError::Protocol(message.into()))?;
    for target_info in target_infos {
        if target_info.target_id == target_id {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Close a top-level target, treating any non-authoritative close response as
/// idempotent success only after the root target inventory proves absence.
async fn close_target_or_confirm_absent(
    conn: &Connection,
    target_id: &str,
) -> Result<(), BrowserError> {
    use chromiumoxide::cdp::browser_protocol::target::CloseTargetParams;

    let original_error = match conn
        .send::<CloseTargetParams>(
            ROOT_SESSION,
            &CloseTargetParams::new(target_id.to_string()),
        )
        .await
    {
        Ok(result)
            if result.get("success").and_then(serde_json::Value::as_bool) == Some(true) =>
        {
            return Ok(());
        }
        Ok(result)
            if result.get("success").and_then(serde_json::Value::as_bool) == Some(false) =>
        {
            BrowserError::Other("Target.closeTarget returned success=false".into())
        }
        Ok(_) => BrowserError::Other(
            "Target.closeTarget response did not contain boolean success=true".into(),
        ),
        Err(error) => map_transport_err(error),
    };

    match target_is_absent_from_browser(conn, target_id).await {
        Ok(true) => Ok(()),
        Ok(false) | Err(_) => Err(original_error),
    }
}

#[derive(Clone)]
struct PendingPage {
    target_id: String,
    session_id: String,
    opener_target_id: Option<String>,
    target_url: Option<String>,
}

#[derive(Clone)]
struct QuarantinedPage {
    pending: PendingPage,
    /// `None` means exact cleanup is already scheduled (or the page is behind
    /// a closing-Lane fence). `Some` is an unowned page's claim grace.
    cleanup_after: Option<tokio::time::Instant>,
}

#[derive(Clone)]
struct TaskTabReservationScope {
    task_resource_key: String,
    lane_id: LaneId,
    authority: Arc<dyn TaskTabReservationAuthority>,
}

#[derive(Clone)]
struct TaskDownloadReservationScope {
    task_resource_key: String,
    lane_id: LaneId,
    authority: Arc<dyn TaskDownloadReservationAuthority>,
}

impl TaskDownloadReservationScope {
    async fn reserve(
        &self,
        download_key: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError> {
        self.authority
            .reserve(&self.task_resource_key, &self.lane_id, download_key)
            .await
    }
}

struct LaneRoute {
    registration_id: LaneRegistrationId,
    tabs: Weak<AsyncMutex<HashMap<String, TabRecord>>>,
    active_target: Weak<AsyncMutex<String>>,
    active_frame: Weak<AsyncMutex<Option<(String, String)>>>,
    closing: Arc<AtomicBool>,
    download_dir: Option<String>,
    task_resource_key: String,
    max_task_tabs: usize,
    task_tab_reservation_scope: Option<TaskTabReservationScope>,
    task_download_reservation_scope: Option<TaskDownloadReservationScope>,
}

struct PendingCreateIntent {
    expires_at: tokio::time::Instant,
    reservation: Option<Arc<dyn TaskTabReservation>>,
    /// Trusted Host-side authority captured before Target.createTarget. The
    /// nonce URL correlates the later attach; page-controlled metadata never
    /// participates in this assignment.
    task_resource_key: Option<String>,
    lane_id: Option<LaneId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LaneRegistrationId(u64);

static NEXT_LANE_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);

impl LaneRegistrationId {
    fn next() -> Self {
        Self(NEXT_LANE_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed))
    }
}

struct PendingDownload {
    lane_id: LaneId,
    download_dir: String,
    suggested_filename: String,
    created_at: std::time::Instant,
    cancel_requested_at: Option<std::time::Instant>,
    reservation: Arc<dyn TaskDownloadReservation>,
}

#[cfg(test)]
struct TestTaskDownloadReservation;

#[cfg(test)]
impl TaskDownloadReservation for TestTaskDownloadReservation {
    fn update_progress(
        &self,
        _received_bytes: u64,
        _total_bytes: Option<u64>,
    ) -> Result<(), BrowserError> {
        Ok(())
    }

    fn prepare_complete(&self, _actual_bytes: u64) -> Result<(), BrowserError> {
        Ok(())
    }

    fn finalize_complete(&self) {}
}

struct HostRouteState {
    ownership: TargetOwnership,
    /// Host-epoch cleanup-only ownership for lanes that already unregistered.
    /// Target ids are never reused within a Chromium epoch, so retaining these
    /// opener tombstones prevents a queued/late popup from escaping after the
    /// Lane's final empty inventory response.
    retired_target_owner: HashMap<String, LaneId>,
    lanes: HashMap<LaneId, LaneRoute>,
    quarantined: HashMap<String, QuarantinedPage>,
    cleanup_inflight: HashSet<String>,
    session_targets: HashMap<String, String>,
    pending_create_urls: HashMap<String, PendingCreateIntent>,
    /// A permit follows its exact top-level target from nonce-correlated
    /// attach through TabRecord publication. Removing this entry alone does
    /// not release the slot once the TabRecord has cloned the same Arc.
    target_tab_reservations: HashMap<String, Arc<dyn TaskTabReservation>>,
    lost_targets: HashMap<String, tokio::time::Instant>,
    frame_owner: HashMap<String, LaneId>,
}

impl HostRouteState {
    fn release_target_bookkeeping(&mut self, target_id: &str, main_frame_id: Option<&str>) {
        self.ownership.release(target_id);
        self.retired_target_owner.remove(target_id);
        self.target_tab_reservations.remove(target_id);
        self.quarantined.remove(target_id);
        self.cleanup_inflight.remove(target_id);
        self.lost_targets.remove(target_id);
        self.session_targets
            .retain(|_, mapped_target| mapped_target != target_id);
        if let Some(main_frame_id) = main_frame_id {
            self.frame_owner.remove(main_frame_id);
        }
    }

    fn quarantine(
        &mut self,
        pending: PendingPage,
        cleanup_after: Option<tokio::time::Instant>,
    ) -> Result<(), ()> {
        if let Some(existing) = self.quarantined.get_mut(&pending.target_id) {
            existing.pending = pending;
            // Never turn an already-scheduled cleanup back into a claimable
            // target because a duplicate attach was delivered.
            if existing.cleanup_after.is_some() {
                existing.cleanup_after = cleanup_after;
            }
            return Ok(());
        }
        // Two short-lived create/attach correlations per registered Lane are
        // allowed in addition to the fixed exception reserve. Thus healthy
        // multi-task concurrency scales with Lane count while one Lane cannot
        // grow the Host staging map without bound.
        let limit = MAX_QUARANTINED_TARGETS.saturating_add(self.lanes.len().saturating_mul(2));
        if self.quarantined.len() >= limit {
            return Err(());
        }
        self.quarantined.insert(
            pending.target_id.clone(),
            QuarantinedPage {
                pending,
                cleanup_after,
            },
        );
        Ok(())
    }

    fn start_cleanup(&mut self, target_id: &str) -> Result<bool, ()> {
        if self.cleanup_inflight.contains(target_id) {
            return Ok(false);
        }
        if self.cleanup_inflight.len() >= MAX_ROUTER_CLEANUP_INFLIGHT {
            return Err(());
        }
        self.cleanup_inflight.insert(target_id.to_string());
        Ok(true)
    }

    fn mark_lost(&mut self, target_id: &str, expires_at: tokio::time::Instant) -> bool {
        if let Some(expiry) = self.lost_targets.get_mut(target_id) {
            *expiry = (*expiry).max(expires_at);
            return false;
        }
        self.lost_targets.insert(target_id.to_string(), expires_at);
        true
    }

    fn active_lane_target_count(&self, lane_id: &str) -> usize {
        self.ownership.targets_for_lane(lane_id).len()
    }

    fn retired_lane_target_count(&self, lane_id: &str) -> usize {
        self.retired_target_owner
            .values()
            .filter(|owner| owner.as_str() == lane_id)
            .count()
    }

    fn effective_task_tab_limit(&self, task_resource_key: &str, candidate: usize) -> usize {
        self.current_task_tab_limit(task_resource_key)
            .unwrap_or(candidate)
    }

    fn current_task_tab_limit(&self, task_resource_key: &str) -> Option<usize> {
        self.lanes
            .values()
            .filter(|route| route.task_resource_key == task_resource_key)
            .map(|route| route.max_task_tabs)
            .min()
    }

    async fn task_tab_count_excluding_lane(
        &self,
        task_resource_key: &str,
        excluded_lane: &str,
    ) -> usize {
        let tabs = self
            .lanes
            .iter()
            .filter(|(lane_id, route)| {
                lane_id.as_str() != excluded_lane
                    && route.task_resource_key == task_resource_key
            })
            .filter_map(|(_, route)| route.tabs.upgrade())
            .collect::<Vec<_>>();
        let mut count = 0usize;
        for tabs in tabs {
            count = count.saturating_add(tabs.lock().await.len());
        }
        count
    }
}

impl Default for HostRouteState {
    fn default() -> Self {
        Self {
            ownership: TargetOwnership::default(),
            retired_target_owner: HashMap::new(),
            lanes: HashMap::new(),
            quarantined: HashMap::new(),
            cleanup_inflight: HashSet::new(),
            session_targets: HashMap::new(),
            pending_create_urls: HashMap::new(),
            target_tab_reservations: HashMap::new(),
            lost_targets: HashMap::new(),
            frame_owner: HashMap::new(),
        }
    }
}

/// Bounded-progress cursor for the dedicated Host download staging directory.
///
/// Keeping the `ReadDir` iterator between reconciliation ticks avoids both an
/// O(directory-size) allocation and the old `read_dir(...).take(512)` restart
/// starvation. One tick advances at most 512 entries; reaching EOF starts a
/// fresh pass on a later tick so newly-created artifacts are still discovered.
struct DownloadStagingScanState {
    entries: Option<std::fs::ReadDir>,
    generation: u64,
    saw_remaining_artifact: bool,
}

impl Default for DownloadStagingScanState {
    fn default() -> Self {
        Self {
            entries: None,
            generation: 0,
            saw_remaining_artifact: false,
        }
    }
}

/// Host-global target router.  It is the sole top-level page discovery loop:
/// explicit target claims win, popups inherit their opener's lane, and targets
/// with neither are retained in quarantine instead of being adopted.
struct HostTargetRouter {
    conn: Connection,
    cleanup_executor: Arc<TargetCleanupExecutor>,
    state: AsyncMutex<HostRouteState>,
    barrier_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<()>>,
    barrier_rx: std::sync::Mutex<
        Option<tokio::sync::mpsc::Receiver<tokio::sync::oneshot::Sender<()>>>,
    >,
    cleanup_changed: tokio::sync::Notify,
    /// Shared with [`DurableProcessCleanup`]: the ledger owns every pending
    /// download route (and its task reservation) and this Host's exclusive
    /// staging directory, so retained download state survives Host Drop until
    /// the exact Chromium process-tree stop has been proven.
    download_ledger: Arc<HostDownloadLedger>,
}

/// Download routing/cleanup state with a lifetime bound to the exact Chromium
/// process, not to the Rust Host object.
///
/// It deliberately holds no connection, router, or process-cleanup reference:
/// the [`DurableProcessCleanup`] completion ticket retains an `Arc` of this
/// ledger and reconciles it after proving the exact process tree stopped.
/// Dropping a `PendingDownload` entry is the single point where an active
/// task download reservation is released.
struct HostDownloadLedger {
    downloads: Mutex<HashMap<String, PendingDownload>>,
    rejected_downloads: Mutex<HashMap<String, std::time::Instant>>,
    /// This exact Host's private browser-level `allowAndName` landing
    /// directory, derived from the trusted root process identity. It is never
    /// a user Downloads directory and never shared with a sibling Host, so
    /// exact GUID artifacts can be reconciled on cancel, timeout, lag, and
    /// shutdown, and the whole directory can be removed after exact stop.
    download_staging_dir: Option<PathBuf>,
    download_cleanup_retries: Mutex<HashSet<PathBuf>>,
    download_staging_scan: Mutex<DownloadStagingScanState>,
    /// A full exact-path table promotes cleanup authority to the dedicated
    /// directory. Generations make completing a scan race-safe: a pass cannot
    /// clear authority published while that pass was running.
    download_directory_cleanup_generation: AtomicU64,
    download_directory_cleanup_completed_generation: AtomicU64,
    download_cleanup_poisoned: AtomicBool,
    /// Sticky admission fence set by the final post-stop drain. A download
    /// loop beat racing that drain must not insert a fresh route (and retain
    /// its reservation forever) after the ledger has been reconciled.
    downloads_finalized: AtomicBool,
}

impl HostDownloadLedger {
    fn new(download_staging_dir: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            downloads: Mutex::new(HashMap::new()),
            rejected_downloads: Mutex::new(HashMap::new()),
            download_staging_dir,
            download_cleanup_retries: Mutex::new(HashSet::new()),
            download_staging_scan: Mutex::new(DownloadStagingScanState::default()),
            download_directory_cleanup_generation: AtomicU64::new(0),
            download_directory_cleanup_completed_generation: AtomicU64::new(0),
            download_cleanup_poisoned: AtomicBool::new(false),
            downloads_finalized: AtomicBool::new(false),
        })
    }
}

/// Runs under exact process-tree stop proof from the durable cleanup relay.
/// This is the only place a retained (cancel-unacknowledged, TTL-expired,
/// lagged, or Host-dropped) download reservation is released.
impl crate::launch::HostStopReconcile for HostDownloadLedger {
    fn reconcile_after_exact_host_stop(&self) {
        let reconciled = self.finalize_downloads_after_host_stop();
        if !reconciled.is_empty() {
            tracing::info!(
                count = reconciled.len(),
                "released retained download reservations after proven exact host stop"
            );
        }
        let residual = self.retry_staging_cleanup();
        if residual == 0 {
            self.remove_exclusive_staging_dir();
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TopLevelTargetLoss {
    Detached,
    Destroyed,
    Crashed,
}

enum AttachedPageRoute {
    Routed(TargetRoute),
    CleanupOnly {
        lane_id: LaneId,
        start_worker: bool,
    },
    EscalateHost,
}

enum ProspectiveTargetOwner {
    Active(LaneId),
    Retired(LaneId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnedPagePublish {
    Inserted,
    AlreadyPresent,
    RejectedCapacity,
    RejectedState,
}

/// Atomic task-tab policy update prepared under the Host router lock.
///
/// Every matching LaneRoute already carries `max_task_tabs` when this value is
/// returned. The caller owns the remaining exact close work and must preserve
/// that stricter cap even if one of those closes fails.
#[derive(Debug)]
pub(crate) struct TaskTabLimitReconcilePlan {
    pub(crate) excess_tabs: Vec<(LaneId, Vec<String>)>,
}

impl TopLevelTargetLoss {
    fn event_name(self) -> &'static str {
        match self {
            Self::Detached => "detached",
            Self::Destroyed => "destroyed",
            Self::Crashed => "crashed",
        }
    }
}

impl HostTargetRouter {
    fn try_new(
        conn: Connection,
        process_cleanup: Option<Arc<DurableProcessCleanup>>,
        download_staging_dir: Option<PathBuf>,
    ) -> Result<Arc<Self>, BrowserError> {
        conn.registry().enable_task_session_quota_routing();
        let (barrier_tx, barrier_rx) = tokio::sync::mpsc::channel(16);
        let cleanup_executor =
            TargetCleanupExecutor::new(conn.clone(), process_cleanup)?;
        Ok(Arc::new(Self {
            conn,
            cleanup_executor,
            state: AsyncMutex::new(HostRouteState::default()),
            barrier_tx,
            barrier_rx: std::sync::Mutex::new(Some(barrier_rx)),
            cleanup_changed: tokio::sync::Notify::new(),
            download_ledger: HostDownloadLedger::new(download_staging_dir),
        }))
    }

    #[cfg(test)]
    fn new(conn: Connection) -> Arc<Self> {
        Self::try_new(conn, None, None).expect("test target cleanup executor starts")
    }

    #[cfg(test)]
    fn new_with_download_staging(conn: Connection, staging: PathBuf) -> Arc<Self> {
        Self::try_new(conn, None, Some(staging))
            .expect("test target cleanup executor starts")
    }

    async fn register_pending_create(
        &self,
        pending_url: &str,
        expires_at: tokio::time::Instant,
        reservation: Option<Arc<dyn TaskTabReservation>>,
        resource_scope: Option<&TaskTabReservationScope>,
    ) -> Result<(), BrowserError> {
        let accepted = {
            let mut state = self.state.lock().await;
            let limit = MAX_PENDING_CREATE_INTENTS
                .saturating_add(state.lanes.len().saturating_mul(2));
            if !state.pending_create_urls.contains_key(pending_url)
                && state.pending_create_urls.len() >= limit
            {
                false
            } else {
                state
                    .pending_create_urls
                    .insert(
                        pending_url.to_string(),
                        PendingCreateIntent {
                            expires_at,
                            reservation,
                            task_resource_key: resource_scope
                                .map(|scope| scope.task_resource_key.clone()),
                            lane_id: resource_scope.map(|scope| scope.lane_id.clone()),
                        },
                    );
                true
            }
        };
        if accepted {
            Ok(())
        } else {
            self.cleanup_executor.poison(None, false);
            Err(BrowserError::SessionLost { recoverable: false })
        }
    }

    async fn register_lane(
        &self,
        lane_id: LaneId,
        tabs: &Arc<AsyncMutex<HashMap<String, TabRecord>>>,
        active_target: &Arc<AsyncMutex<String>>,
        active_frame: &Arc<AsyncMutex<Option<(String, String)>>>,
        closing: Arc<AtomicBool>,
        download_dir: Option<String>,
    ) -> Option<LaneRegistrationId> {
        self.register_lane_with_resource_scope(
            lane_id,
            tabs,
            active_target,
            active_frame,
            closing,
            download_dir,
            None,
            usize::MAX,
            None,
        )
        .await
    }

    async fn register_lane_with_resource_scope(
        &self,
        lane_id: LaneId,
        tabs: &Arc<AsyncMutex<HashMap<String, TabRecord>>>,
        active_target: &Arc<AsyncMutex<String>>,
        active_frame: &Arc<AsyncMutex<Option<(String, String)>>>,
        closing: Arc<AtomicBool>,
        download_dir: Option<String>,
        task_resource_key: Option<String>,
        max_task_tabs: usize,
        task_tab_reservation_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    ) -> Option<LaneRegistrationId> {
        self.register_lane_with_resource_and_download_scope(
            lane_id,
            tabs,
            active_target,
            active_frame,
            closing,
            download_dir,
            task_resource_key,
            max_task_tabs,
            task_tab_reservation_authority,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn register_lane_with_resource_and_download_scope(
        &self,
        lane_id: LaneId,
        tabs: &Arc<AsyncMutex<HashMap<String, TabRecord>>>,
        active_target: &Arc<AsyncMutex<String>>,
        active_frame: &Arc<AsyncMutex<Option<(String, String)>>>,
        closing: Arc<AtomicBool>,
        download_dir: Option<String>,
        task_resource_key: Option<String>,
        max_task_tabs: usize,
        task_tab_reservation_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
        task_download_reservation_authority: Option<
            Arc<dyn TaskDownloadReservationAuthority>,
        >,
    ) -> Option<LaneRegistrationId> {
        let registration_id = LaneRegistrationId::next();
        let mut state = self.state.lock().await;
        if state.lanes.contains_key(&lane_id) {
            return None;
        }
        let task_resource_key = task_resource_key.unwrap_or_else(|| lane_id.clone());
        let task_tab_reservation_scope = task_tab_reservation_authority.map(|authority| {
            TaskTabReservationScope {
                task_resource_key: task_resource_key.clone(),
                lane_id: lane_id.clone(),
                authority,
            }
        });
        let task_download_reservation_scope =
            task_download_reservation_authority.map(|authority| TaskDownloadReservationScope {
                task_resource_key: task_resource_key.clone(),
                lane_id: lane_id.clone(),
                authority,
            });
        let existing_task_tabs = state
            .task_tab_count_excluding_lane(&task_resource_key, &lane_id)
            .await;
        let incoming_tabs = tabs.lock().await.len();
        let effective_limit =
            state.effective_task_tab_limit(&task_resource_key, max_task_tabs);
        if incoming_tabs > MAX_TABS_PER_LANE
            || existing_task_tabs.saturating_add(incoming_tabs) > effective_limit
        {
            tracing::warn!(
                target: "nomi_browser_engine::host",
                lane_id = %lane_id,
                task_tab_count = existing_task_tabs.saturating_add(incoming_tabs),
                task_tab_limit = effective_limit,
                "refused to register a browser Lane beyond its task tab budget"
            );
            return None;
        }
        state.lanes.insert(
            lane_id,
            LaneRoute {
                registration_id,
                tabs: Arc::downgrade(tabs),
                active_target: Arc::downgrade(active_target),
                active_frame: Arc::downgrade(active_frame),
                closing,
                download_dir,
                task_resource_key,
                // Existing routes are the live Host policy authority. A Lane
                // launch snapshot may be stale in either direction: a higher
                // value must not undo lowering, and a lower value must not
                // undo a completed raise. Dynamic policy changes go through
                // prepare_task_tab_limit_reconciliation, never registration.
                max_task_tabs: effective_limit,
                task_tab_reservation_scope,
                task_download_reservation_scope,
            },
        );
        Some(registration_id)
    }

    /// Atomically install a task-wide tab limit on every live route in this
    /// Host and select deterministic non-active tabs which must be closed.
    ///
    /// One capacity slot is reserved for every Lane, including a temporarily
    /// empty Lane which may need to recover its final crashed target. A caller
    /// must therefore prune whole excess Lanes before requesting a limit below
    /// the current Lane count.
    async fn prepare_task_tab_limit_reconciliation(
        &self,
        task_resource_key: &str,
        max_task_tabs: usize,
    ) -> Result<TaskTabLimitReconcilePlan, BrowserError> {
        if max_task_tabs == 0 {
            return Err(BrowserError::Blocked {
                reason: "a browser task tab limit must retain at least one page".into(),
            });
        }

        let mut state = self.state.lock().await;
        let mut lane_ids = state
            .lanes
            .iter()
            .filter_map(|(lane_id, route)| {
                (route.task_resource_key == task_resource_key).then(|| lane_id.clone())
            })
            .collect::<Vec<_>>();
        lane_ids.sort();
        if lane_ids.is_empty() {
            return Ok(TaskTabLimitReconcilePlan {
                excess_tabs: Vec::new(),
            });
        }
        if max_task_tabs < lane_ids.len() {
            return Err(BrowserError::Blocked {
                reason: format!(
                    "the task tab limit {max_task_tabs} is below its {} live browser Lanes; close excess Lanes first",
                    lane_ids.len()
                ),
            });
        }

        let mut lane_targets = Vec::with_capacity(lane_ids.len());
        let mut survivors = HashSet::with_capacity(max_task_tabs);
        let mut additional_candidates = Vec::new();
        for lane_id in &lane_ids {
            let Some(route) = state.lanes.get(lane_id) else {
                return Err(BrowserError::TargetClosed);
            };
            if route.closing.load(Ordering::Acquire) {
                return Err(BrowserError::TargetClosed);
            }
            let Some(tabs) = route.tabs.upgrade() else {
                return Err(BrowserError::TargetClosed);
            };
            let Some(active_target) = route.active_target.upgrade() else {
                return Err(BrowserError::TargetClosed);
            };
            let Some(active_frame) = route.active_frame.upgrade() else {
                return Err(BrowserError::TargetClosed);
            };

            let mut target_ids = tabs.lock().await.keys().cloned().collect::<Vec<_>>();
            target_ids.sort();
            if target_ids.is_empty() {
                return Err(BrowserError::Blocked {
                    reason: format!(
                        "browser Lane {lane_id} has no live top-level page; recover or close it before lowering the task tab limit"
                    ),
                });
            }
            let mut active_target = active_target.lock().await;
            let current_active = active_target.clone();
            let selected_active = target_ids
                .binary_search(&current_active)
                .is_ok()
                .then_some(current_active)
                .or_else(|| target_ids.first().cloned());
            if let Some(selected_active) = selected_active.as_ref() {
                survivors.insert(selected_active.clone());
                if active_target.as_str() != selected_active {
                    *active_target = selected_active.clone();
                    *active_frame.lock().await = None;
                }
            }
            drop(active_target);
            for target_id in &target_ids {
                if Some(target_id) != selected_active.as_ref() {
                    additional_candidates.push((lane_id.clone(), target_id.clone()));
                }
            }
            lane_targets.push((lane_id.clone(), target_ids));
        }

        additional_candidates.sort();
        let additional_capacity = max_task_tabs.saturating_sub(lane_ids.len());
        for (_, target_id) in additional_candidates.into_iter().take(additional_capacity) {
            survivors.insert(target_id);
        }

        let mut excess_tabs = Vec::new();
        for (lane_id, target_ids) in lane_targets {
            let excess = target_ids
                .into_iter()
                .filter(|target_id| !survivors.contains(target_id))
                .collect::<Vec<_>>();
            if !excess.is_empty() {
                excess_tabs.push((lane_id, excess));
            }
        }

        // This is the policy commit point. It occurs only after every route
        // and weak Lane handle was validated, and before any target close I/O.
        // A failed close must leave this stricter admission limit installed.
        for lane_id in lane_ids {
            let Some(route) = state.lanes.get_mut(&lane_id) else {
                return Err(BrowserError::TargetClosed);
            };
            route.max_task_tabs = max_task_tabs;
        }
        Ok(TaskTabLimitReconcilePlan { excess_tabs })
    }

    #[cfg(test)]
    async fn task_tab_limit(&self, task_resource_key: &str) -> Option<usize> {
        let state = self.state.lock().await;
        state
            .lanes
            .values()
            .filter(|route| route.task_resource_key == task_resource_key)
            .map(|route| route.max_task_tabs)
            .min()
    }

    async fn task_lane_ids(&self, task_resource_key: &str) -> Vec<LaneId> {
        let state = self.state.lock().await;
        let mut lane_ids = state
            .lanes
            .iter()
            .filter_map(|(lane_id, route)| {
                (route.task_resource_key == task_resource_key).then(|| lane_id.clone())
            })
            .collect::<Vec<_>>();
        lane_ids.sort();
        lane_ids
    }

    async fn task_tab_count(&self, task_resource_key: &str) -> usize {
        let tabs = {
            let state = self.state.lock().await;
            state
                .lanes
                .values()
                .filter(|route| route.task_resource_key == task_resource_key)
                .filter_map(|route| route.tabs.upgrade())
                .collect::<Vec<_>>()
        };
        let mut count = 0usize;
        for tabs in tabs {
            count = count.saturating_add(tabs.lock().await.len());
        }
        count
    }

    async fn has_task_tab_reservation(&self, target_id: &str) -> bool {
        self.state
            .lock()
            .await
            .target_tab_reservations
            .contains_key(target_id)
    }

    /// Claim a target for one Lane. Returns `false` when the target already
    /// crashed or belongs to another Lane.
    async fn claim_target(self: &Arc<Self>, lane_id: &str, target_id: &str) -> bool {
        let (pending_pages, saturated) = {
            let mut state = self.state.lock().await;
            if state.lost_targets.contains_key(target_id) {
                return false;
            }
            if state.cleanup_inflight.contains(target_id) {
                return false;
            }
            let Some(route) = state.lanes.get(lane_id) else {
                return false;
            };
            if route.closing.load(Ordering::Acquire) {
                return false;
            }
            let mut tracked_targets = state.ownership.targets_for_lane(lane_id).len();
            let target_already_owned = state.ownership.owner(target_id) == Some(lane_id);
            if !target_already_owned
                && tracked_targets >= MAX_TRACKED_TARGETS_PER_LANE
            {
                tracing::error!(
                    target: "nomi_browser_engine::host",
                    lane_id = %lane_id,
                    target_limit = MAX_TRACKED_TARGETS_PER_LANE,
                    "Lane target ownership exceeded its bounded target inventory"
                );
                drop(state);
                self.cleanup_executor.poison(None, false);
                return false;
            }
            if let Err(owner) = state.ownership.claim(lane_id, target_id) {
                tracing::warn!(
                    target: "nomi_browser_engine::host",
                    target_id_suffix = %cdp_id_suffix(target_id),
                    requested_lane = %lane_id,
                    established_lane = %owner,
                    "refused to transfer an owned target between lanes"
                );
                return false;
            }
            if !target_already_owned {
                tracked_targets += 1;
            }
            let mut pages = Vec::new();
            let mut inherited_from = vec![target_id.to_string()];
            if let Some(page) = state.quarantined.remove(target_id) {
                pages.push(page.pending);
            }
            let mut saturated = false;
            'inheritance: while let Some(opener_id) = inherited_from.pop() {
                let children = state
                    .quarantined
                    .iter()
                    .filter_map(|(id, page)| {
                        (page.pending.opener_target_id.as_deref() == Some(opener_id.as_str()))
                            .then_some(id.clone())
                    })
                    .collect::<Vec<_>>();
                for child_id in children {
                    let child_already_owned =
                        state.ownership.owner(&child_id) == Some(lane_id);
                    if !child_already_owned
                        && tracked_targets >= MAX_TRACKED_TARGETS_PER_LANE
                    {
                        saturated = true;
                        break 'inheritance;
                    }
                    if state.ownership.claim(lane_id, &child_id).is_ok()
                        && let Some(page) = state.quarantined.remove(&child_id)
                    {
                        if !child_already_owned {
                            tracked_targets += 1;
                        }
                        inherited_from.push(child_id);
                        pages.push(page.pending);
                    }
                }
            }
            (pages, saturated)
        };
        if saturated {
            tracing::error!(
                target: "nomi_browser_engine::host",
                lane_id = %lane_id,
                target_limit = MAX_TRACKED_TARGETS_PER_LANE,
                "popup target inheritance exceeded its bounded Lane inventory"
            );
            self.cleanup_executor.poison(None, false);
            return false;
        }
        for pending in pending_pages {
            self.arm_owned_page(lane_id.to_string(), pending).await;
        }
        true
    }

    async fn is_target_lost(&self, target_id: &str) -> bool {
        self.state.lock().await.lost_targets.contains_key(target_id)
    }

    async fn owned_targets(&self, lane_id: &str) -> Vec<String> {
        self.state.lock().await.ownership.targets_for_lane(lane_id)
    }

    async fn is_current_registration(
        &self,
        lane_id: &str,
        registration_id: LaneRegistrationId,
    ) -> bool {
        self.state
            .lock()
            .await
            .lanes
            .get(lane_id)
            .is_some_and(|route| route.registration_id == registration_id)
    }

    /// Discover live top-level descendants of a closing Lane directly from the
    /// root target inventory and claim them into that Lane's cleanup set.
    ///
    /// This is a pre-unregister drain, not an event-ordering barrier: targets
    /// already visible in Chromium are claimed here, while the Host-epoch
    /// `retired_target_owner` fence handles attaches materialized or delivered
    /// only after the Lane unregisters.
    fn map_lane_inventory_error(
        &self,
        lane_id: &str,
        error: BoundedLaneTargetInventoryError,
    ) -> BrowserError {
        if matches!(error, BoundedLaneTargetInventoryError::Malformed(_)) {
            return BrowserError::Other(error.to_string());
        }
        tracing::error!(
            target: "nomi_browser_engine::host",
            lane_id = %lane_id,
            %error,
            "bounded Lane cleanup inventory was exceeded; escalating exact Host cleanup"
        );
        // The retired-Lane tombstones remain authoritative until the exact
        // Host cleanup proof completes. Poisoning prevents this malformed Host
        // from accumulating one indefinitely retrying finalizer per Lane.
        self.cleanup_executor.poison(None, false);
        BrowserError::SessionLost { recoverable: false }
    }

    async fn claim_live_targets_for_closing_lane(
        &self,
        lane_id: &str,
    ) -> Result<Vec<String>, BrowserError> {
        let seed_targets = {
            let state = self.state.lock().await;
            let Some(lane) = state.lanes.get(lane_id) else {
                return Err(BrowserError::Other(
                    "closing Lane disappeared before target drain".into(),
                ));
            };
            if !lane.closing.load(Ordering::Acquire) {
                return Err(BrowserError::Other(
                    "target drain requires a closing Lane".into(),
                ));
            }
            state.ownership.targets_for_lane(lane_id)
        };
        let result = self
            .conn
            .send::<GetTargetsParams>(ROOT_SESSION, &GetTargetsParams::default())
            .await
            .map_err(map_transport_err)?;
        let lineage = bounded_lane_seed_targets(seed_targets.iter().map(String::as_str))
            .map_err(|error| self.map_lane_inventory_error(lane_id, error))?;
        let live_targets = bounded_live_lane_targets(&result, lineage)
            .map_err(|error| self.map_lane_inventory_error(lane_id, error))?;

        let mut state = self.state.lock().await;
        for target_id in &live_targets {
            if let Err(established_lane) =
                state.ownership.claim(lane_id.to_string(), target_id.clone())
                && established_lane != lane_id
            {
                return Err(BrowserError::Other(
                    "live target lineage conflicts with another Lane owner".into(),
                ));
            }
        }
        Ok(live_targets)
    }

    async fn unregister_lane_if_current(
        &self,
        lane_id: &str,
        registration_id: LaneRegistrationId,
    ) -> bool {
        let mut state = self.state.lock().await;
        if state
            .lanes
            .get(lane_id)
            .is_none_or(|route| route.registration_id != registration_id)
        {
            return false;
        }
        state.lanes.remove(lane_id);
        let released = state.ownership.release_lane(lane_id);
        for target_id in released {
            state
                .retired_target_owner
                .insert(target_id.clone(), lane_id.to_string());
            state.lost_targets.remove(&target_id);
            state
                .session_targets
                .retain(|_, mapped_target| mapped_target != &target_id);
        }
        state.frame_owner.retain(|_, owner| owner != lane_id);
        drop(state);
        let retired_guids = {
            let mut downloads = self
                .download_ledger
                .downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let now = std::time::Instant::now();
            downloads
                .iter_mut()
                .filter_map(|(guid, route)| {
                    if route.lane_id == lane_id && route.cancel_requested_at.is_none() {
                        route.cancel_requested_at = Some(now);
                        Some(guid.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for guid in &retired_guids {
            if !cancel_download_best_effort(&self.conn, guid, "owning browser Lane closed").await {
                self.download_ledger
                    .download_cleanup_poisoned
                    .store(true, Ordering::Release);
                self.conn.shutdown().await;
                break;
            }
        }
        true
    }

    #[cfg(test)]
    async fn unregister_lane(&self, lane_id: &str) {
        let registration_id = self
            .state
            .lock()
            .await
            .lanes
            .get(lane_id)
            .map(|route| route.registration_id);
        if let Some(registration_id) = registration_id {
            let _ = self
                .unregister_lane_if_current(lane_id, registration_id)
                .await;
        }
    }

    /// Forget router bookkeeping for a target which was created for a Lane
    /// launch but was closed before that Lane could be registered.
    ///
    /// The exact target id comes from the nonce-correlated create/attach
    /// transaction. Never scrub a target which another Lane managed to claim.
    async fn scrub_unowned_pending_target(
        &self,
        target_id: &str,
        session_id: Option<&str>,
        expected_reservation: Option<&Arc<dyn TaskTabReservation>>,
    ) {
        let mut state = self.state.lock().await;
        if state.ownership.owner(target_id).is_some()
            || state.retired_target_owner.contains_key(target_id)
        {
            return;
        }
        state.quarantined.remove(target_id);
        state.cleanup_inflight.remove(target_id);
        state.lost_targets.remove(target_id);
        if expected_reservation.is_some_and(|expected| {
            state
                .target_tab_reservations
                .get(target_id)
                .is_some_and(|current| Arc::ptr_eq(current, expected))
        }) {
            state.target_tab_reservations.remove(target_id);
        }
        state
            .session_targets
            .retain(|session, mapped_target| {
                mapped_target != target_id && session_id != Some(session.as_str())
            });
        self.cleanup_changed.notify_waiters();
    }

    /// Remove exact residual records after a cancelled Lane launch has proved
    /// its target absent. Generation matching is handled by
    /// `unregister_lane_if_current`; this exact scrub therefore cannot remove a
    /// later same-id Lane registration.
    async fn scrub_cancelled_lane_target(
        &self,
        lane_id: &str,
        target_id: &str,
        session_id: &str,
        frame_id: &str,
        exact_absence_proven: bool,
        expected_reservation: Option<&Arc<dyn TaskTabReservation>>,
    ) {
        let mut state = self.state.lock().await;
        // A cancelled initial-Lane launch can be closed without Chromium ever
        // delivering Target.targetDestroyed. Release the router's permit only
        // after the exact close/absence proof, and only when it is still the
        // same reservation captured by this cleanup generation. A late cleanup
        // must never remove a replacement reservation stored under the same
        // target key.
        if exact_absence_proven
            && expected_reservation.is_some_and(|expected| {
                state
                    .target_tab_reservations
                    .get(target_id)
                    .is_some_and(|current| Arc::ptr_eq(current, expected))
            })
        {
            state.target_tab_reservations.remove(target_id);
        }
        state.quarantined.remove(target_id);
        state.cleanup_inflight.remove(target_id);
        state.lost_targets.remove(target_id);
        state
            .session_targets
            .retain(|session, mapped_target| {
                mapped_target != target_id && session != session_id
            });
        if state.frame_owner.get(frame_id).map(String::as_str) == Some(lane_id) {
            state.frame_owner.remove(frame_id);
        }
        self.cleanup_changed.notify_waiters();
    }

    async fn release_target(&self, target_id: &str, main_frame_id: Option<&str>) {
        let mut state = self.state.lock().await;
        state.release_target_bookkeeping(target_id, main_frame_id);
        self.cleanup_changed.notify_waiters();
    }

    async fn schedule_owned_target_cleanup(
        self: &Arc<Self>,
        lane_id: &str,
        target_id: &str,
    ) {
        let start_worker = {
            let mut state = self.state.lock().await;
            state.start_cleanup(target_id)
        };
        match start_worker {
            Ok(true) => self.cleanup_executor.submit(TargetCleanupJob::RouterTarget {
                router: Arc::clone(self),
                lane_id: lane_id.to_string(),
                target_id: target_id.to_string(),
            }),
            Ok(false) => {}
            Err(()) => self.cleanup_executor.poison(None, false),
        }
    }

    async fn claim_frame(&self, lane_id: &str, frame_id: &str) {
        let mut state = self.state.lock().await;
        match state.frame_owner.get(frame_id) {
            Some(owner) if owner != lane_id => {
                tracing::warn!(
                    %frame_id,
                    requested_lane = %lane_id,
                    established_lane = %owner,
                    "refused to transfer a frame between browser lanes"
                );
            }
            _ => {
                state
                    .frame_owner
                    .insert(frame_id.to_string(), lane_id.to_string());
            }
        }
    }

    async fn begin_download(
        &self,
        frame_id: &str,
        guid: &str,
        suggested_filename: &str,
    ) -> bool {
        if guid.is_empty()
            || guid.len() > crate::session::MAX_CDP_IDENTIFIER_BYTES
            || suggested_filename.is_empty()
            || suggested_filename.len() > MAX_DOWNLOAD_SUGGESTED_FILENAME_BYTES
        {
            return false;
        }
        if self
            .download_ledger
            .download_cleanup_poisoned
            .load(Ordering::Acquire)
        {
            return false;
        }
        if !download_capacity_available(self.pending_download_count()) {
            tracing::warn!(
                %guid,
                limit = MAX_PENDING_DOWNLOADS_PER_HOST,
                "download routing capacity exhausted; admission denied"
            );
            return false;
        }
        let lane_id = {
            let state = self.state.lock().await;
            state
                .frame_owner
                .get(frame_id)
                .cloned()
                .or_else(|| state.ownership.owner(frame_id).map(str::to_string))
        };
        // F21：frame_owner/ownership 只登记主帧 id（== page target id）；iframe
        //（同进程或 OOPIF）内发起的下载带的是**子帧** frameId，两张表恒查不到。
        // 未命中时按各 lane 活 tab 的 frame tree 解析归属（CDP roundtrip 仅在
        // 子帧下载这一低频路径发生）。
        let lane_id = match lane_id {
            Some(lane_id) => Some(lane_id),
            None => self.resolve_subframe_download_owner(frame_id).await,
        };
        let Some(lane_id) = lane_id else {
            tracing::warn!(
                %guid,
                %frame_id,
                "download has no owned frame; admission denied"
            );
            return false;
        };
        let (download_dir, download_scope, registration_id) = {
            let state = self.state.lock().await;
            let Some(route) = state.lanes.get(&lane_id) else {
                return false;
            };
            (
                route.download_dir.clone(),
                route.task_download_reservation_scope.clone(),
                route.registration_id,
            )
        };
        let Some(download_dir) = download_dir else {
            return false;
        };
        let reservation: Arc<dyn TaskDownloadReservation> = match download_scope {
            Some(scope) => match scope.reserve(guid).await {
                Ok(reservation) => reservation,
                Err(error) => {
                    tracing::warn!(%guid, lane_id = %lane_id, %error, "task download admission denied");
                    return false;
                }
            },
            #[cfg(test)]
            None => Arc::new(TestTaskDownloadReservation),
            #[cfg(not(test))]
            None => return false,
        };
        // Reservation can await a cross-Host authority. Revalidate that the
        // exact Lane registration still owns this route before publication.
        {
            let state = self.state.lock().await;
            if !state
                .lanes
                .get(&lane_id)
                .is_some_and(|route| route.registration_id == registration_id)
            {
                return false;
            }
        }
        let mut downloads = self
            .download_ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Re-check the sticky fences under the routes lock: the final drain
        // fences before draining, so a reservation obtained while that drain
        // was racing is dropped here (releasing its active charge) instead of
        // being retained past exact host stop.
        if self
            .download_ledger
            .download_cleanup_poisoned
            .load(Ordering::Acquire)
            || self
                .download_ledger
                .downloads_finalized
                .load(Ordering::Acquire)
        {
            return false;
        }
        // Repeated GUIDs cannot replace an existing routing authority.  A
        // duplicate (or a race at the hard cap) is denied fail-closed.
        if downloads.contains_key(guid) || !download_capacity_available(downloads.len()) {
            return false;
        }
        downloads.insert(
            guid.to_string(),
            PendingDownload {
                lane_id,
                download_dir,
                suggested_filename: suggested_filename.to_string(),
                created_at: std::time::Instant::now(),
                cancel_requested_at: None,
                reservation,
            },
        );
        true
    }

    /// F21：把子帧 frameId 解析到其所属 lane。遍历各 lane 的活 [`TabRecord`]，查
    /// page session 的 frameTree（同进程 iframe）与 OOPIF 子 session 的 frameTree
    /// （跨进程 iframe，其根 frame id 即 downloadWillBegin 携带的子帧 id）。
    ///
    /// 锁纪律：短锁克隆句柄（lane 表 → tabs 表 → oopif 表逐层克隆后立即放锁），
    /// CDP I/O（`frame_ids`）全在锁外。单帧树查询失败（session 已关竞态）跳过——
    /// 解析不到即维持隔离检疫（fail-closed：文件留在 Host staging，绝不误配 lane）。
    async fn resolve_subframe_download_owner(&self, frame_id: &str) -> Option<LaneId> {
        let lanes: Vec<(LaneId, Arc<AsyncMutex<HashMap<String, TabRecord>>>)> = {
            let state = self.state.lock().await;
            state
                .lanes
                .iter()
                .filter_map(|(lane_id, route)| {
                    route.tabs.upgrade().map(|tabs| (lane_id.clone(), tabs))
                })
                .collect()
        };
        for (lane_id, tabs) in lanes {
            let (mut managers, oopif_tables) = {
                let tabs = tabs.lock().await;
                let mut managers = Vec::new();
                let mut oopif_tables = Vec::new();
                for record in tabs.values() {
                    managers.push(record.injection.clone());
                    oopif_tables.push(Arc::clone(&record.oopif_managers));
                }
                (managers, oopif_tables)
            };
            for table in oopif_tables {
                let table = table
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                managers.extend(table.values().map(|entry| entry.manager.clone()));
            }
            for manager in managers {
                if let Ok(frame_ids) = manager.frame_ids().await
                    && frame_ids.iter().any(|candidate| candidate == frame_id)
                {
                    return Some(lane_id);
                }
            }
        }
        None
    }
}

impl HostDownloadLedger {
    fn update_download_progress(
        &self,
        guid: &str,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> bool {
        let downloads = self
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(route) = downloads.get(guid) else {
            return false;
        };
        match route
            .reservation
            .update_progress(received_bytes, total_bytes)
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    %guid,
                    lane_id = %route.lane_id,
                    %error,
                    "download crossed its task byte boundary; cancelling"
                );
                false
            }
        }
    }

    fn finish_download(&self, guid: &str, source: &std::path::Path) -> bool {
        let route = self
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(guid);
        let Some(route) = route else {
            self.cancel_pending_download(guid);
            return false;
        };
        // The task-lifetime charge uses the artifact's actual on-disk size;
        // CDP-reported totals are page-influenced and never trusted here.
        let actual_bytes = match std::fs::metadata(source) {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            Ok(_) => {
                self.cleanup_staged_download(guid, Some(source));
                return false;
            }
            Err(error) => {
                tracing::warn!(%guid, file = %source.display(), %error, "completed download metadata is unavailable");
                self.cleanup_staged_download(guid, Some(source));
                return false;
            }
        };
        let filename = std::path::Path::new(&route.suggested_filename)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(guid);
        // Two-phase compensating transaction: prepare the final charge, stage
        // into a unique same-volume temp, publish atomically without
        // clobbering, then finalize with no intervening await. A failed
        // publication rolls the charge back (or keeps it, fail-closed, when
        // the residual artifact cannot be deleted).
        match crate::download::publish_task_output(
            route.reservation.as_ref(),
            actual_bytes,
            crate::download::TaskOutputPayload::StagedFile(source),
            std::path::Path::new(&route.download_dir),
            filename,
            guid,
        ) {
            Ok(destination) => {
                if let Err(error) = crate::download::write_motw(&destination) {
                    tracing::debug!(
                        %error,
                        file = %destination.display(),
                        "MOTW write failed after lane download routing"
                    );
                }
                // Reconcile any cross-volume staging leftover.
                self.cleanup_staged_download(guid, Some(source));
                true
            }
            Err(failure) => {
                tracing::warn!(
                    %guid,
                    lane_id = %route.lane_id,
                    charged = failure.charged,
                    error = %failure.message,
                    "failed to publish completed download to its owning lane"
                );
                if let Some(residual) = failure.residual {
                    // We created this temp file, so deletion authority is
                    // ours; the bounded retry table keeps converging on it.
                    self.cleanup_or_retain_staging_path(residual);
                }
                self.cleanup_staged_download(guid, Some(source));
                false
            }
        }
    }

    fn pending_download_count(&self) -> usize {
        self.downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Request cancellation for timed-out routes without releasing their task
    /// reservations. A cancel acknowledgement is not a terminal proof.
    fn expire_pending_downloads(&self) -> Vec<String> {
        let now = std::time::Instant::now();
        self.downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter_mut()
            .filter_map(|(guid, route)| {
                if route.cancel_requested_at.is_none()
                    && now.saturating_duration_since(route.created_at) >= DOWNLOAD_ROUTE_TTL
                {
                    route.cancel_requested_at = Some(now);
                    Some(guid.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn cancel_terminal_grace_expired(&self) -> bool {
        let now = std::time::Instant::now();
        let active_expired = self
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|route| {
                route.cancel_requested_at.is_some_and(|requested_at| {
                    now.saturating_duration_since(requested_at)
                        >= DOWNLOAD_CANCEL_TERMINAL_GRACE
                })
            });
        let rejected_expired = self
            .rejected_downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|requested_at| {
                now.saturating_duration_since(*requested_at)
                    >= DOWNLOAD_CANCEL_TERMINAL_GRACE
            });
        if active_expired || rejected_expired {
            self.download_cleanup_poisoned
                .store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    fn cancel_pending_download(&self, guid: &str) {
        self.downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(guid);
        self.rejected_downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(guid);
        self.cleanup_staged_download(guid, None);
    }

    /// Preserve cleanup and task-reservation authority for a rejected download
    /// until Chromium publishes a terminal event or exact Host stop is proven.
    fn quarantine_rejected_download(&self, guid: &str) -> bool {
        let now = std::time::Instant::now();
        if let Some(route) = self
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(guid)
        {
            route.cancel_requested_at.get_or_insert(now);
            return true;
        }
        let mut rejected = self
            .rejected_downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !rejected.contains_key(guid)
            && rejected.len() >= MAX_QUARANTINED_DOWNLOADS_PER_HOST
        {
            self.download_cleanup_poisoned
                .store(true, Ordering::Release);
            return false;
        }
        rejected.entry(guid.to_string()).or_insert(now);
        true
    }

    /// Fence every retained download after progress observability is lost.
    /// Reservations remain held until exact Host/process stop.
    fn poison_downloads_for_host_stop(&self) -> Vec<String> {
        self.download_cleanup_poisoned
            .store(true, Ordering::Release);
        let now = std::time::Instant::now();
        let mut guids = self
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter_mut()
            .map(|(guid, route)| {
                route.cancel_requested_at.get_or_insert(now);
                guid.clone()
            })
            .collect::<Vec<_>>();
        guids.extend(
            self.rejected_downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .keys()
                .cloned(),
        );
        guids.sort();
        guids.dedup();
        guids
    }

    /// Final bounded drain. Callers must already hold exact Host/process-stop
    /// proof; connection closure or cancel acknowledgement is insufficient.
    fn finalize_downloads_after_host_stop(&self) -> Vec<String> {
        // Sticky admission fence first: a download-loop beat racing this
        // drain must not insert a fresh route (and retain its reservation
        // forever) after the ledger has been reconciled.
        self.downloads_finalized.store(true, Ordering::Release);
        let mut guids = self
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .map(|(guid, _)| guid)
            .collect::<Vec<_>>();
        guids.extend(
            self.rejected_downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drain()
                .map(|(guid, _)| guid),
        );
        guids.sort();
        guids.dedup();
        self.cleanup_staged_guids(&guids);
        guids
    }

    fn download_cancel_requested(&self, guid: &str) -> bool {
        self.downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(guid)
            .is_some_and(|route| route.cancel_requested_at.is_some())
    }

    fn cleanup_staged_guids(&self, guids: &[String]) {
        for guid in guids {
            self.cleanup_staged_download(guid, None);
        }
    }

    fn cleanup_staged_download(&self, guid: &str, event_path: Option<&std::path::Path>) {
        let Some(staging_dir) = self.download_staging_dir.as_deref() else {
            return;
        };
        // Never remove an arbitrary CDP-supplied path.  Only a direct child of
        // the configured Host staging directory is cleanup-authorized.
        if let Some(path) = event_path
            && path.parent() == Some(staging_dir)
        {
            self.cleanup_or_retain_staging_path(path.to_path_buf());
        }
        let Some(name) = safe_download_guid_component(guid) else {
            return;
        };
        self.cleanup_or_retain_staging_path(staging_dir.join(name));
        self.cleanup_or_retain_staging_path(staging_dir.join(format!("{name}.crdownload")));
        self.cleanup_or_retain_staging_path(staging_dir.join(format!("{name}.tmp")));
    }

    fn cleanup_or_retain_staging_path(&self, path: PathBuf) -> bool {
        if remove_download_staging_file(&path) {
            self.download_cleanup_retries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&path);
            return true;
        }
        let mut retries = self
            .download_cleanup_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retries.len() < MAX_DOWNLOAD_CLEANUP_RETRIES || retries.contains(&path) {
            retries.insert(path);
        } else {
            // The staging directory is a dedicated, durable cleanup boundary,
            // so one generation is sufficient authority for any number of
            // direct-child paths which no longer fit the exact retry table.
            // The rotating scanner below eventually visits every child and a
            // future Host resumes from the same on-disk directory after crash.
            self.download_directory_cleanup_generation
                .fetch_add(1, Ordering::AcqRel);
            tracing::error!(
                limit = MAX_DOWNLOAD_CLEANUP_RETRIES,
                file = %path.display(),
                "download staging exact retry table exhausted; promoted cleanup to directory authority"
            );
            self.download_cleanup_poisoned
                .store(true, Ordering::Release);
        }
        false
    }

    fn retry_staging_cleanup(&self) -> usize {
        let paths = self
            .download_cleanup_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for path in paths {
            if remove_download_staging_file(&path) {
                self.download_cleanup_retries
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&path);
            }
        }
        let exact = self.download_cleanup_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        exact.saturating_add(usize::from(self.directory_cleanup_pending()))
    }

    /// Startup/periodic safety net for a prior process that died after Chrome
    /// created an `allowAndName` file but before an event route existed. This is
    /// a dedicated Host staging directory, never a user Downloads directory, so
    /// every old direct-child file is cleanup-authorized. That directory-level
    /// rule also survives process death when an exact in-memory path table was
    /// saturated.
    fn sweep_stale_staging_files(&self) {
        self.sweep_stale_staging_files_at(std::time::SystemTime::now());
    }

    fn directory_cleanup_pending(&self) -> bool {
        self.download_directory_cleanup_generation
            .load(Ordering::Acquire)
            > self
                .download_directory_cleanup_completed_generation
                .load(Ordering::Acquire)
    }

    fn sweep_stale_staging_files_at(&self, now: std::time::SystemTime) {
        let Some(staging_dir) = self.download_staging_dir.as_deref() else {
            return;
        };
        // Active and retained-cancel routes still own their staging artifacts:
        // a cancel acknowledgement or TTL expiry is not terminal proof, so a
        // stalled-but-owned GUID file must never be swept by age alone.
        let owned_guids = {
            let downloads = self
                .downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let rejected = self
                .rejected_downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            downloads
                .keys()
                .cloned()
                .chain(rejected.keys().cloned())
                .collect::<HashSet<String>>()
        };
        let mut scan = self
            .download_staging_scan
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if scan.entries.is_none() {
            let Ok(entries) = std::fs::read_dir(staging_dir) else {
                return;
            };
            scan.entries = Some(entries);
            scan.generation = self
                .download_directory_cleanup_generation
                .load(Ordering::Acquire);
            scan.saw_remaining_artifact = false;
        }

        let mut completed_pass = false;
        for _ in 0..MAX_DOWNLOAD_STAGING_SCAN_ENTRIES {
            let next = scan
                .entries
                .as_mut()
                .expect("download staging iterator initialized")
                .next();
            let entry = match next {
                Some(Ok(entry)) => entry,
                Some(Err(_)) => {
                    if self.directory_cleanup_pending() {
                        scan.saw_remaining_artifact = true;
                    }
                    continue;
                }
                None => {
                    completed_pass = true;
                    scan.entries = None;
                    break;
                }
            };
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    if self.directory_cleanup_pending() {
                        scan.saw_remaining_artifact = true;
                    }
                    continue;
                }
            };
            if !metadata.is_file() {
                if self.directory_cleanup_pending() {
                    // A direct-child directory is not a Chromium artifact and
                    // is never deleted recursively, but it keeps promoted
                    // directory authority visibly pending rather than letting
                    // a synthetic or corrupted staging entry hide cleanup.
                    scan.saw_remaining_artifact = true;
                }
                continue;
            }
            let owned = entry
                .file_name()
                .to_str()
                .map(|name| {
                    let stem = name
                        .strip_suffix(".crdownload")
                        .or_else(|| name.strip_suffix(".tmp"))
                        .unwrap_or(name);
                    owned_guids.contains(stem)
                })
                .unwrap_or(false);
            if owned {
                if self.directory_cleanup_pending() {
                    scan.saw_remaining_artifact = true;
                }
                continue;
            }
            let old_enough = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age >= DOWNLOAD_ROUTE_TTL);
            if !old_enough {
                if self.directory_cleanup_pending() {
                    scan.saw_remaining_artifact = true;
                }
                continue;
            }
            if !self.cleanup_or_retain_staging_path(entry.path()) {
                scan.saw_remaining_artifact = true;
            }
        }

        if completed_pass {
            let current_generation = self
                .download_directory_cleanup_generation
                .load(Ordering::Acquire);
            if current_generation == scan.generation && !scan.saw_remaining_artifact {
                self.download_directory_cleanup_completed_generation
                    .store(current_generation, Ordering::Release);
            }
        }
    }

    /// Removes this exact Host's private staging directory once reconcile
    /// left no residual cleanup debt. The path was derived from the exact
    /// root process identity, so no sibling Host can own files inside it.
    fn remove_exclusive_staging_dir(&self) {
        let Some(staging_dir) = self.download_staging_dir.as_deref() else {
            return;
        };
        if let Err(error) = std::fs::remove_dir_all(staging_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(
                dir = %staging_dir.display(),
                %error,
                "exclusive download staging directory removal deferred to the orphan sweep"
            );
        }
    }
}

impl HostTargetRouter {
    fn update_download_progress(
        &self,
        guid: &str,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> bool {
        self.download_ledger
            .update_download_progress(guid, received_bytes, total_bytes)
    }

    async fn finish_download(&self, guid: &str, source: &std::path::Path) -> bool {
        self.download_ledger.finish_download(guid, source)
    }

    fn pending_download_count(&self) -> usize {
        self.download_ledger.pending_download_count()
    }

    fn expire_pending_downloads(&self) -> Vec<String> {
        self.download_ledger.expire_pending_downloads()
    }

    fn cancel_terminal_grace_expired(&self) -> bool {
        self.download_ledger.cancel_terminal_grace_expired()
    }

    fn cancel_pending_download(&self, guid: &str) {
        self.download_ledger.cancel_pending_download(guid);
    }

    fn quarantine_rejected_download(&self, guid: &str) -> bool {
        self.download_ledger.quarantine_rejected_download(guid)
    }

    fn poison_downloads_for_host_stop(&self) -> Vec<String> {
        self.download_ledger.poison_downloads_for_host_stop()
    }

    fn finalize_downloads_after_host_stop(&self) -> Vec<String> {
        self.download_ledger.finalize_downloads_after_host_stop()
    }

    fn download_cancel_requested(&self, guid: &str) -> bool {
        self.download_ledger.download_cancel_requested(guid)
    }

    fn cleanup_staged_download(&self, guid: &str, event_path: Option<&std::path::Path>) {
        self.download_ledger.cleanup_staged_download(guid, event_path);
    }

    fn retry_staging_cleanup(&self) -> usize {
        self.download_ledger.retry_staging_cleanup()
    }

    fn sweep_stale_staging_files(&self) {
        self.download_ledger.sweep_stale_staging_files();
    }

    async fn handle_attached(self: &Arc<Self>, pending: PendingPage) {
        if self.cleanup_executor.ensure_accepting().is_err() {
            return;
        }
        let (route, session_authority) = {
            let mut state = self.state.lock().await;
            if state.lost_targets.contains_key(&pending.target_id) {
                return;
            }
            state.session_targets.retain(|session_id, mapped_target| {
                mapped_target != &pending.target_id || session_id == &pending.session_id
            });
            state
                .session_targets
                .insert(pending.session_id.clone(), pending.target_id.clone());
            let correlated_create = pending
                .target_url
                .as_deref()
                .and_then(|url| state.pending_create_urls.remove(url));
            if let Some(reservation) = correlated_create
                .as_ref()
                .and_then(|intent| intent.reservation.as_ref())
            {
                state
                    .target_tab_reservations
                    .entry(pending.target_id.clone())
                    .or_insert_with(|| Arc::clone(reservation));
            }
            let correlated_session_authority = correlated_create.as_ref().and_then(|intent| {
                Some((
                    intent.task_resource_key.clone()?,
                    intent.lane_id.clone()?,
                ))
            });
            let correlated_create_deadline = correlated_create.as_ref().map(|_| {
                tokio::time::Instant::now() + PENDING_PAGE_CREATE_RECOVERY_TIMEOUT
            });
            let prospective_owner = state
                .ownership
                .owner(&pending.target_id)
                .map(str::to_owned)
                .map(ProspectiveTargetOwner::Active)
                .or_else(|| {
                    state
                        .retired_target_owner
                        .get(&pending.target_id)
                        .cloned()
                        .map(ProspectiveTargetOwner::Retired)
                })
                .or_else(|| {
                    pending
                        .opener_target_id
                        .as_deref()
                        .and_then(|opener_id| state.ownership.owner(opener_id))
                        .map(str::to_owned)
                        .map(ProspectiveTargetOwner::Active)
                })
                .or_else(|| {
                    pending
                        .opener_target_id
                        .as_deref()
                        .and_then(|opener_id| state.retired_target_owner.get(opener_id))
                        .cloned()
                        .map(ProspectiveTargetOwner::Retired)
                });
            let cleanup_owner = match prospective_owner.as_ref() {
                Some(ProspectiveTargetOwner::Active(lane_id))
                    if state
                        .lanes
                        .get(lane_id)
                        .is_some_and(|lane| lane.closing.load(Ordering::Acquire)) =>
                {
                    Some((lane_id.clone(), false))
                }
                Some(ProspectiveTargetOwner::Retired(lane_id)) => {
                    Some((lane_id.clone(), true))
                }
                _ => None,
            };
            let route = if let Some((lane_id, retired)) = cleanup_owner {
                if retired {
                    let target_already_retired = state
                        .retired_target_owner
                        .get(&pending.target_id)
                        .is_some_and(|owner| owner == &lane_id);
                    if !target_already_retired
                        && state.retired_lane_target_count(&lane_id)
                            >= MAX_TRACKED_TARGETS_PER_LANE
                    {
                        state.session_targets.remove(&pending.session_id);
                        AttachedPageRoute::EscalateHost
                    } else {
                        state
                            .retired_target_owner
                            .insert(pending.target_id.clone(), lane_id.clone());
                        if state.quarantine(pending.clone(), None).is_err() {
                            state.session_targets.remove(&pending.session_id);
                            AttachedPageRoute::EscalateHost
                        } else {
                            match state.start_cleanup(&pending.target_id) {
                                Ok(start_worker) => AttachedPageRoute::CleanupOnly {
                                    lane_id,
                                    start_worker,
                                },
                                Err(()) => AttachedPageRoute::EscalateHost,
                            }
                        }
                    }
                } else {
                    let target_already_owned =
                        state.ownership.owner(&pending.target_id) == Some(lane_id.as_str());
                    if !target_already_owned
                        && state.active_lane_target_count(&lane_id)
                            >= MAX_TRACKED_TARGETS_PER_LANE
                    {
                        state.session_targets.remove(&pending.session_id);
                        AttachedPageRoute::EscalateHost
                    } else if let Err(established_lane) = state
                        .ownership
                        .claim(lane_id.clone(), pending.target_id.clone())
                        && established_lane != lane_id
                    {
                        tracing::warn!(
                            target: "nomi_browser_engine::host",
                            target_id_suffix = %cdp_id_suffix(&pending.target_id),
                            requested_lane = %lane_id,
                            established_lane = %established_lane,
                            "late target cleanup ownership conflicts with another Lane"
                        );
                        state.session_targets.remove(&pending.session_id);
                        AttachedPageRoute::EscalateHost
                    } else if state.quarantine(pending.clone(), None).is_err() {
                        state.session_targets.remove(&pending.session_id);
                        AttachedPageRoute::EscalateHost
                    } else {
                        match state.start_cleanup(&pending.target_id) {
                            Ok(start_worker) => AttachedPageRoute::CleanupOnly {
                                lane_id,
                                start_worker,
                            },
                            Err(()) => AttachedPageRoute::EscalateHost,
                        }
                    }
                }
            } else {
                let active_owner = match prospective_owner.as_ref() {
                    Some(ProspectiveTargetOwner::Active(lane_id)) => Some(lane_id.clone()),
                    _ => None,
                };
                let target_already_owned = active_owner.as_deref().is_some_and(|lane_id| {
                    state.ownership.owner(&pending.target_id) == Some(lane_id)
                });
                if let Some(lane_id) = active_owner.as_deref()
                    && !target_already_owned
                    && state.active_lane_target_count(lane_id) >= MAX_TRACKED_TARGETS_PER_LANE
                {
                    state.session_targets.remove(&pending.session_id);
                    AttachedPageRoute::EscalateHost
                } else {
                    let route = state.ownership.route_attached(
                        &pending.target_id,
                        pending.opener_target_id.as_deref(),
                    );
                    if route == TargetRoute::Quarantined {
                        let cleanup_after = correlated_create_deadline.unwrap_or_else(|| {
                            tokio::time::Instant::now() + QUARANTINED_TARGET_GRACE
                        });
                        if state
                            .quarantine(pending.clone(), Some(cleanup_after))
                            .is_err()
                        {
                            state.session_targets.remove(&pending.session_id);
                            AttachedPageRoute::EscalateHost
                        } else {
                            AttachedPageRoute::Routed(route)
                        }
                    } else {
                        AttachedPageRoute::Routed(route)
                    }
                }
            };
            let routed_session_authority = match &route {
                AttachedPageRoute::Routed(
                    TargetRoute::Owned(lane_id) | TargetRoute::Inherited { lane_id, .. },
                )
                | AttachedPageRoute::CleanupOnly { lane_id, .. } => state
                    .lanes
                    .get(lane_id)
                    .map(|lane| (lane.task_resource_key.clone(), lane_id.clone())),
                AttachedPageRoute::Routed(TargetRoute::Quarantined)
                | AttachedPageRoute::EscalateHost => None,
            };
            (
                route,
                correlated_session_authority.or(routed_session_authority),
            )
        };
        if let Some((task_resource_key, lane_id)) = session_authority
            && let Err(error) = self.conn.registry().claim_task_session_authority(
                &pending.session_id,
                &task_resource_key,
                &lane_id,
            )
        {
            tracing::error!(
                target: "nomi_browser_engine::host",
                lane_id = %lane_id,
                target_id_suffix = %cdp_id_suffix(&pending.target_id),
                %error,
                "trusted page-session authority conflicted; escalating exact Host cleanup"
            );
            self.cleanup_executor.poison(None, false);
            return;
        }
        match route {
            AttachedPageRoute::Routed(
                TargetRoute::Owned(lane_id) | TargetRoute::Inherited { lane_id, .. },
            ) => {
                self.arm_owned_page(lane_id, pending).await;
            }
            AttachedPageRoute::Routed(TargetRoute::Quarantined) => {
                tracing::debug!(
                    target: "nomi_browser_engine::host",
                    target_id_suffix = %cdp_id_suffix(&pending.target_id),
                    opener_target_id_suffix = ?pending
                        .opener_target_id
                        .as_deref()
                        .map(cdp_id_suffix),
                    "top-level target quarantined until an owning lane claims it"
                );
            }
            AttachedPageRoute::CleanupOnly {
                lane_id,
                start_worker,
            } => {
                tracing::debug!(
                    target: "nomi_browser_engine::host",
                    lane_id = %lane_id,
                    target_id_suffix = %cdp_id_suffix(&pending.target_id),
                    "late top-level target retained behind a Lane cleanup-only fence"
                );
                if start_worker {
                    self.cleanup_executor.submit(TargetCleanupJob::RouterTarget {
                        router: Arc::clone(self),
                        lane_id,
                        target_id: pending.target_id,
                    });
                }
            }
            AttachedPageRoute::EscalateHost => {
                tracing::error!(
                    target: "nomi_browser_engine::host",
                    target_id_suffix = %cdp_id_suffix(&pending.target_id),
                    quarantine_base_limit = MAX_QUARANTINED_TARGETS,
                    cleanup_limit = MAX_ROUTER_CLEANUP_INFLIGHT,
                    "Host target router state saturated or ownership conflicted; escalating to authoritative Host cleanup"
                );
                self.cleanup_executor.poison(None, false);
            }
        }
    }

    async fn retry_cleanup_only_target(&self, lane_id: Option<LaneId>, target_id: String) {
        let mut attempt = 0usize;
        loop {
            attempt += 1;
            match close_target_or_confirm_absent(&self.conn, &target_id).await {
                Ok(()) => {
                    let active_owned = {
                        let state = self.state.lock().await;
                        lane_id.as_deref().is_some_and(|lane_id| {
                            state.ownership.owner(&target_id) == Some(lane_id)
                                && state.lanes.contains_key(lane_id)
                        })
                    };
                    if active_owned {
                        self.handle_top_level_target_loss(
                            Some(target_id.clone()),
                            None,
                            TopLevelTargetLoss::Destroyed,
                        )
                        .await;
                    }
                    let mut state = self.state.lock().await;
                    if !state.retired_target_owner.contains_key(&target_id) {
                        state.ownership.release(&target_id);
                        state.lost_targets.remove(&target_id);
                    }
                    state.quarantined.remove(&target_id);
                    state.cleanup_inflight.remove(&target_id);
                    state.target_tab_reservations.remove(&target_id);
                    state
                        .session_targets
                        .retain(|_, mapped_target| mapped_target != &target_id);
                    drop(state);
                    self.cleanup_changed.notify_waiters();
                    return;
                }
                Err(error) if self.conn.registry().is_connection_closed() => {
                    let mut state = self.state.lock().await;
                    state.cleanup_inflight.remove(&target_id);
                    state.quarantined.remove(&target_id);
                    state
                        .session_targets
                        .retain(|_, mapped_target| mapped_target != &target_id);
                    drop(state);
                    self.cleanup_changed.notify_waiters();
                    tracing::warn!(
                        target: "nomi_browser_engine::host",
                        lane_id = ?lane_id,
                        target_id_suffix = %cdp_id_suffix(&target_id),
                        %error,
                        attempts = attempt,
                        "late target cleanup handed to authoritative Host shutdown"
                    );
                    return;
                }
                Err(error) => {
                    if attempt == 20 || attempt % 60 == 0 {
                        tracing::warn!(
                            target: "nomi_browser_engine::host",
                            lane_id = ?lane_id,
                            target_id_suffix = %cdp_id_suffix(&target_id),
                            %error,
                            attempts = attempt,
                            "late target cleanup is still retrying under retained Host-epoch ownership"
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    async fn sweep_transient_target_state(self: &Arc<Self>) {
        if self.cleanup_executor.ensure_accepting().is_err() {
            return;
        }
        let now = tokio::time::Instant::now();
        let (expired_quarantine, saturated) = {
            let mut state = self.state.lock().await;

            state
                .pending_create_urls
                .retain(|_, intent| intent.expires_at > now);

            let expired_lost = state
                .lost_targets
                .iter()
                .filter_map(|(target_id, expires_at)| {
                    (*expires_at <= now).then(|| target_id.clone())
                })
                .collect::<Vec<_>>();
            for target_id in expired_lost {
                state.lost_targets.remove(&target_id);
                if !state.retired_target_owner.contains_key(&target_id) {
                    state.ownership.release(&target_id);
                }
            }

            let expired = state
                .quarantined
                .iter()
                .filter_map(|(target_id, page)| {
                    page.cleanup_after
                        .is_some_and(|deadline| deadline <= now)
                        .then(|| target_id.clone())
                })
                .collect::<Vec<_>>();
            let mut workers = Vec::with_capacity(expired.len());
            let mut saturated = false;
            for target_id in expired {
                if let Some(page) = state.quarantined.get_mut(&target_id) {
                    page.cleanup_after = None;
                }
                match state.start_cleanup(&target_id) {
                    Ok(true) => workers.push(target_id),
                    Ok(false) => {}
                    Err(()) => {
                        saturated = true;
                        break;
                    }
                }
            }
            (workers, saturated)
        };

        if saturated {
            tracing::error!(
                target: "nomi_browser_engine::host",
                cleanup_limit = MAX_ROUTER_CLEANUP_INFLIGHT,
                "expired target quarantine saturated cleanup bookkeeping; escalating Host cleanup"
            );
            self.cleanup_executor.poison(None, false);
            return;
        }
        for target_id in expired_quarantine {
            self.cleanup_executor
                .submit(TargetCleanupJob::QuarantinedTarget {
                    router: Arc::clone(self),
                    target_id,
                });
        }
    }

    async fn event_barrier(&self) -> Result<(), BrowserError> {
        self.event_barrier_with_timeout(HOST_ROUTER_BARRIER_TIMEOUT)
            .await
    }

    async fn event_barrier_with_timeout(&self, timeout: Duration) -> Result<(), BrowserError> {
        tokio::time::timeout(timeout, async {
            let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
            self.barrier_tx
                .send(ack_tx)
                .await
                .map_err(|_| BrowserError::SessionLost { recoverable: false })?;
            ack_rx
                .await
                .map_err(|_| BrowserError::SessionLost { recoverable: false })
        })
        .await
        .map_err(|_| {
            BrowserError::Other(
                "Host target router event barrier timed out during Lane cleanup".into(),
            )
        })?
    }

    async fn claim_live_retired_targets(
        self: &Arc<Self>,
        lane_id: &str,
    ) -> Result<Vec<String>, BrowserError> {
        let seed_targets = {
            let state = self.state.lock().await;
            bounded_lane_seed_targets(state.retired_target_owner.iter().filter_map(
                |(target_id, owner)| (owner == lane_id).then_some(target_id.as_str()),
            ))
        }
        .map_err(|error| self.map_lane_inventory_error(lane_id, error))?;
        let result = self
            .conn
            .send::<GetTargetsParams>(ROOT_SESSION, &GetTargetsParams::default())
            .await
            .map_err(map_transport_err)?;
        let live_targets = bounded_live_lane_targets(&result, seed_targets)
            .map_err(|error| self.map_lane_inventory_error(lane_id, error))?;
        let (workers, saturated) = {
            let mut state = self.state.lock().await;
            let mut workers = Vec::new();
            let mut saturated = false;
            for target_id in &live_targets {
                if let Some(active_owner) = state.ownership.owner(target_id)
                    && active_owner != lane_id
                {
                    return Err(BrowserError::Other(
                        "retired target lineage conflicts with an active sibling Lane".into(),
                    ));
                }
                state
                    .retired_target_owner
                    .insert(target_id.clone(), lane_id.to_string());
                match state.start_cleanup(target_id) {
                    Ok(true) => workers.push(target_id.clone()),
                    Ok(false) => {}
                    Err(()) => {
                        saturated = true;
                        break;
                    }
                }
            }
            (workers, saturated)
        };
        if saturated {
            self.cleanup_executor.poison(None, false);
            return Err(BrowserError::SessionLost { recoverable: false });
        }
        for target_id in workers {
            self.cleanup_executor.submit(TargetCleanupJob::RouterTarget {
                router: Arc::clone(self),
                lane_id: lane_id.to_string(),
                target_id,
            });
        }
        Ok(live_targets)
    }

    async fn wait_retired_cleanup_idle(
        &self,
        lane_id: &str,
        timeout: Duration,
    ) -> Result<(), BrowserError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.cleanup_changed.notified();
            let pending = {
                let state = self.state.lock().await;
                state.cleanup_inflight.iter().any(|target_id| {
                    state
                        .retired_target_owner
                        .get(target_id)
                        .is_some_and(|owner| owner == lane_id)
                })
            };
            if !pending {
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(BrowserError::Other(
                    "retired Lane target cleanup did not reach an absent proof".into(),
                ));
            }
        }
    }

    async fn finalize_retired_lane(self: &Arc<Self>, lane_id: &str) -> Result<(), BrowserError> {
        const MAX_FINALIZE_PASSES: usize = 8;
        const CLEANUP_WAIT: Duration = Duration::from_secs(5);

        let mut consecutive_empty = 0usize;
        for _ in 0..MAX_FINALIZE_PASSES {
            let live_targets = self.claim_live_retired_targets(lane_id).await?;
            // Target.getTargets response establishes the transport ordering
            // point. The biased router mailbox barrier then drains every
            // target event already delivered before that response.
            self.event_barrier().await?;
            self.wait_retired_cleanup_idle(lane_id, CLEANUP_WAIT)
                .await?;
            if live_targets.is_empty() {
                consecutive_empty += 1;
                if consecutive_empty == 2 {
                    self.clear_retired_lane_bookkeeping(lane_id).await;
                    return Ok(());
                }
            } else {
                consecutive_empty = 0;
            }
        }
        Err(BrowserError::Other(
            "retired Lane target inventory did not stabilize empty".into(),
        ))
    }

    async fn clear_retired_lane_bookkeeping(&self, lane_id: &str) {
        let mut state = self.state.lock().await;
        let targets = state
            .retired_target_owner
            .iter()
            .filter_map(|(target_id, owner)| (owner == lane_id).then(|| target_id.clone()))
            .collect::<Vec<_>>();
        for target_id in &targets {
            state.retired_target_owner.remove(target_id);
            state.ownership.release(target_id);
            state.quarantined.remove(target_id);
            state.cleanup_inflight.remove(target_id);
            state.lost_targets.remove(target_id);
            state.target_tab_reservations.remove(target_id);
        }
        state
            .session_targets
            .retain(|_, target_id| !targets.contains(target_id));
        drop(state);
        self.cleanup_changed.notify_waiters();
    }

    /// Publish a fully armed page under the Host router lock. This is the one
    /// atomic admission point for both popup discovery and caller-created tabs:
    /// sibling Lanes with the same task key are counted before the record is
    /// inserted, so concurrent Lanes cannot each pass a stale pre-check.
    async fn publish_armed_page(
        self: &Arc<Self>,
        lane_id: &str,
        pending: PendingPage,
        record: TabRecord,
    ) -> OwnedPagePublish {
        let mut state = self.state.lock().await;
        if state.cleanup_inflight.contains(&pending.target_id) {
            abort_tab_record(&record);
            return OwnedPagePublish::RejectedCapacity;
        }
        if state.lost_targets.contains_key(&pending.target_id)
            || state.ownership.owner(&pending.target_id) != Some(lane_id)
        {
            abort_tab_record(&record);
            return OwnedPagePublish::RejectedState;
        }
        let Some((tabs, closing, task_resource_key, max_task_tabs)) = state
            .lanes
            .get(lane_id)
            .map(|route| {
                (
                    route.tabs.clone(),
                    route.closing.clone(),
                    route.task_resource_key.clone(),
                    route.max_task_tabs,
                )
            })
        else {
            abort_tab_record(&record);
            return OwnedPagePublish::RejectedState;
        };
        if closing.load(Ordering::Acquire) {
            abort_tab_record(&record);
            return OwnedPagePublish::RejectedState;
        }
        let Some(tabs) = tabs.upgrade() else {
            abort_tab_record(&record);
            return OwnedPagePublish::RejectedState;
        };

        let sibling_task_tabs = state
            .task_tab_count_excluding_lane(&task_resource_key, lane_id)
            .await;
        let effective_task_limit =
            state.effective_task_tab_limit(&task_resource_key, max_task_tabs);
        let mut tabs = tabs.lock().await;
        if tabs.contains_key(&pending.target_id) {
            abort_tab_record(&record);
            return OwnedPagePublish::AlreadyPresent;
        }
        let task_tabs_after_insert = sibling_task_tabs
            .saturating_add(tabs.len())
            .saturating_add(1);
        if !tab_capacity_available(tabs.len())
            || task_tabs_after_insert > effective_task_limit
        {
            abort_tab_record(&record);
            let quarantined = state.quarantine(pending.clone(), None);
            let start_worker = quarantined
                .as_ref()
                .map(|()| state.start_cleanup(&pending.target_id))
                .unwrap_or(Err(()));
            drop(tabs);
            drop(state);

            tracing::warn!(
                target: "nomi_browser_engine::host",
                lane_id = %lane_id,
                lane_tab_limit = MAX_TABS_PER_LANE,
                task_tab_count = task_tabs_after_insert,
                task_tab_limit = effective_task_limit,
                target_id_suffix = %cdp_id_suffix(&pending.target_id),
                "closed excess top-level target to preserve Lane and task tab bounds"
            );
            match start_worker {
                Ok(true) => self.cleanup_executor.submit(TargetCleanupJob::RouterTarget {
                    router: Arc::clone(self),
                    lane_id: lane_id.to_string(),
                    target_id: pending.target_id,
                }),
                Ok(false) => {}
                Err(()) => self.cleanup_executor.poison(None, false),
            }
            return OwnedPagePublish::RejectedCapacity;
        }

        let main_frame_id = record.main_frame_id.clone();
        tabs.insert(pending.target_id.clone(), record);
        drop(tabs);
        drop(state);
        self.claim_frame(lane_id, &main_frame_id).await;
        OwnedPagePublish::Inserted
    }

    async fn arm_owned_page(self: &Arc<Self>, lane_id: LaneId, pending: PendingPage) {
        let route = {
            let state = self.state.lock().await;
            if state.lost_targets.contains_key(&pending.target_id) {
                return;
            }
            state
                .lanes
                .get(&lane_id)
                .map(|route| {
                    (
                        route.tabs.clone(),
                        route.closing.clone(),
                        route.task_resource_key.clone(),
                        route.task_tab_reservation_scope.clone(),
                        state
                            .target_tab_reservations
                            .get(&pending.target_id)
                            .cloned(),
                    )
                })
        };
        let Some((
            tabs,
            closing,
            task_resource_key,
            reservation_scope,
            existing_reservation,
        )) = route
        else {
            return;
        };
        if closing.load(Ordering::Acquire) {
            return;
        }
        match self.conn.registry().claim_task_session_authority(
            &pending.session_id,
            &task_resource_key,
            &lane_id,
        ) {
            Ok(TaskSessionAdmission::Admitted) => {}
            Ok(TaskSessionAdmission::PendingAuthority | TaskSessionAdmission::Rejected) => {
                self.schedule_owned_target_cleanup(&lane_id, &pending.target_id)
                    .await;
                return;
            }
            Err(error) => {
                tracing::error!(
                    target: "nomi_browser_engine::host",
                    lane_id = %lane_id,
                    target_id_suffix = %cdp_id_suffix(&pending.target_id),
                    %error,
                    "owned page session could not acquire trusted task/Lane authority"
                );
                self.cleanup_executor.poison(None, false);
                return;
            }
        }
        let Some(tabs) = tabs.upgrade() else {
            return;
        };
        if tabs.lock().await.contains_key(&pending.target_id) {
            return;
        }

        let reservation = match (existing_reservation, reservation_scope) {
            (Some(reservation), _) => Some(reservation),
            (None, Some(scope)) => {
                let reservation = match scope
                    .authority
                    .reserve(
                        &scope.task_resource_key,
                        &scope.lane_id,
                        &pending.target_id,
                    )
                    .await
                {
                    Ok(reservation) => reservation,
                    Err(error) => {
                        tracing::warn!(
                            target: "nomi_browser_engine::host",
                            lane_id = %lane_id,
                            target_id_suffix = %cdp_id_suffix(&pending.target_id),
                            %error,
                            "task-wide tab reservation rejected a top-level target"
                        );
                        self.schedule_owned_target_cleanup(&lane_id, &pending.target_id)
                            .await;
                        return;
                    }
                };
                let mut state = self.state.lock().await;
                let route_is_current = state
                    .lanes
                    .get(&lane_id)
                    .is_some_and(|route| {
                        !route.closing.load(Ordering::Acquire)
                            && state.ownership.owner(&pending.target_id) == Some(lane_id.as_str())
                    });
                if !route_is_current || state.lost_targets.contains_key(&pending.target_id) {
                    drop(state);
                    self.schedule_owned_target_cleanup(&lane_id, &pending.target_id)
                        .await;
                    return;
                }
                Some(
                    state
                        .target_tab_reservations
                        .entry(pending.target_id.clone())
                        .or_insert(reservation)
                        .clone(),
                )
            }
            (None, None) => None,
        };

        let deadline = tokio::time::Instant::now() + OOPIF_SESSION_REGISTER_TIMEOUT;
        while !self.conn.registry().has_session(&pending.session_id) {
            if closing.load(Ordering::Acquire) || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        match arm_tab(
            &self.conn,
            &pending.target_id,
            &pending.session_id,
            reservation,
        )
        .await
        {
            Ok(record) => {
                if self
                    .publish_armed_page(&lane_id, pending.clone(), record)
                    .await
                    == OwnedPagePublish::Inserted
                {
                    tracing::info!(
                        target: "nomi_browser_engine::host",
                        lane_id = %lane_id,
                        target_id_suffix = %cdp_id_suffix(&pending.target_id),
                        "top-level target assigned to browser lane"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "nomi_browser_engine::host",
                    lane_id = %lane_id,
                    target_id_suffix = %cdp_id_suffix(&pending.target_id),
                    %error,
                    "failed to arm lane-owned target"
                );
                self.schedule_owned_target_cleanup(&lane_id, &pending.target_id)
                    .await;
            }
        }
    }

    /// Remove exactly one lost top-level target from its owning Lane.
    ///
    /// Ownership is resolved before touching Lane state, so a renderer crash
    /// or detach can never fan out into another Lane. The target id remains an
    /// short reorder tombstone in `TargetOwnership` so an in-flight arm cannot
    /// resurrect the dead target. Only sessions recorded by this router are
    /// eligible for session-based loss, which excludes worker and OOPIF
    /// sessions from the top-level Lane registry.
    async fn handle_top_level_target_loss(
        &self,
        target_id: Option<String>,
        session_id: Option<String>,
        loss: TopLevelTargetLoss,
    ) {
        let route = {
            let mut state = self.state.lock().await;
            let target_id = target_id.or_else(|| {
                session_id
                    .as_deref()
                    .and_then(|session_id| state.session_targets.get(session_id).cloned())
            });
            let Some(target_id) = target_id else {
                return;
            };
            let active_owner = state.ownership.owner(&target_id).map(str::to_owned);
            let tracked_top_level = active_owner.is_some()
                || state.retired_target_owner.contains_key(&target_id)
                || state.quarantined.contains_key(&target_id)
                || state
                    .session_targets
                    .values()
                    .any(|mapped_target| mapped_target == &target_id);
            if !tracked_top_level {
                return;
            }
            if active_owner.is_some() {
                let expires_at =
                    tokio::time::Instant::now() + LOST_TARGET_TOMBSTONE_GRACE;
                if !state.mark_lost(&target_id, expires_at) {
                    return;
                }
            }
            state.quarantined.remove(&target_id);
            // Only Target.targetDestroyed is a physical-page terminal proof.
            // Session detach/crash events are routed through exact target
            // cleanup and never reach this release point directly.
            state.target_tab_reservations.remove(&target_id);
            if state.cleanup_inflight.remove(&target_id) {
                self.cleanup_changed.notify_waiters();
            }
            state
                .session_targets
                .retain(|_, mapped_target| mapped_target != &target_id);
            let Some(lane_id) = active_owner else {
                return;
            };
            let Some(lane) = state.lanes.get(&lane_id) else {
                return;
            };
            (
                target_id,
                lane_id,
                lane.tabs.clone(),
                lane.active_target.clone(),
                lane.active_frame.clone(),
            )
        };

        let (target_id, lane_id, tabs, active_target, active_frame) = route;
        let (Some(tabs), Some(active_target), Some(active_frame)) = (
            tabs.upgrade(),
            active_target.upgrade(),
            active_frame.upgrade(),
        ) else {
            return;
        };

        let removed = tabs.lock().await.remove(&target_id);
        if let Some(record) = removed {
            let main_frame_id = record.main_frame_id.clone();
            abort_tab_record(&record);
            self.state.lock().await.frame_owner.remove(&main_frame_id);
        }

        let was_active = active_target.lock().await.as_str() == target_id;
        let survivor = if was_active {
            deterministic_survivor(
                tabs.lock().await.keys().map(String::as_str),
                &target_id,
            )
        } else {
            None
        };
        if was_active {
            *active_target.lock().await = survivor.clone().unwrap_or_default();
            *active_frame.lock().await = None;
        }

        tracing::warn!(
            target: "nomi_browser_engine::host",
            lane_id = %lane_id,
            target_id_suffix = %cdp_id_suffix(&target_id),
            survivor_target_id_suffix = ?survivor.as_deref().map(cdp_id_suffix),
            loss = loss.event_name(),
            "top-level target was lost; only its owning browser lane was updated"
        );
    }

    /// A renderer/session loss is not proof that its top-level Target ceased
    /// to exist. Preserve the TabRecord, ownership and task reservation, then
    /// submit exactly one bounded close/absence job. Repeated detach/crash
    /// events coalesce through `cleanup_inflight`.
    async fn handle_top_level_session_loss(
        self: &Arc<Self>,
        target_id: Option<String>,
        session_id: Option<String>,
        loss: TopLevelTargetLoss,
    ) {
        let cleanup = {
            let mut state = self.state.lock().await;
            let target_id = target_id.or_else(|| {
                session_id
                    .as_deref()
                    .and_then(|session_id| state.session_targets.get(session_id).cloned())
            });
            let Some(target_id) = target_id else {
                return;
            };
            // `Target.targetDestroyed` is stronger, physical terminal proof.
            // Its short-lived tombstone must suppress a later queued
            // detach/crash event; otherwise the stale event could mint a new
            // cleanup job for an already absent target until ownership's
            // grace-period sweep runs.
            if state.lost_targets.contains_key(&target_id) {
                return;
            }
            let lane_id = state
                .ownership
                .owner(&target_id)
                .map(str::to_owned)
                .or_else(|| state.retired_target_owner.get(&target_id).cloned());
            let tracked_top_level = lane_id.is_some()
                || state.quarantined.contains_key(&target_id)
                || state.target_tab_reservations.contains_key(&target_id)
                || state
                    .session_targets
                    .values()
                    .any(|mapped_target| mapped_target == &target_id);
            if !tracked_top_level {
                return;
            }
            match state.start_cleanup(&target_id) {
                Ok(true) => Some((target_id, lane_id)),
                Ok(false) => None,
                Err(()) => {
                    drop(state);
                    self.cleanup_executor.poison(None, false);
                    return;
                }
            }
        };

        if let Some((target_id, lane_id)) = cleanup {
            tracing::warn!(
                target: "nomi_browser_engine::host",
                lane_id = ?lane_id,
                target_id_suffix = %cdp_id_suffix(&target_id),
                loss = loss.event_name(),
                "top-level target session was lost; retaining its task slot until exact target cleanup"
            );
            match lane_id {
                Some(lane_id) => self.cleanup_executor.submit(TargetCleanupJob::RouterTarget {
                    router: Arc::clone(self),
                    lane_id,
                    target_id,
                }),
                None => self
                    .cleanup_executor
                    .submit(TargetCleanupJob::QuarantinedTarget {
                        router: Arc::clone(self),
                        target_id,
                    }),
            }
        }
    }

    fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let router = self.clone();
        let mut barrier_rx = self
            .barrier_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("Host target router is spawned exactly once");
        let mut attached_rx = router
            .conn
            .subscribe_reliable(EventAttachedToTarget::IDENTIFIER, None);
        let mut detached_rx = router
            .conn
            .subscribe_reliable(EventDetachedFromTarget::IDENTIFIER, None);
        let mut destroyed_rx = router
            .conn
            .subscribe_reliable(EventTargetDestroyed::IDENTIFIER, None);
        let mut crashed_rx = router
            .conn
            .subscribe_reliable(EventTargetCrashed::IDENTIFIER, None);
        let mut inspector_detached_rx = router
            .conn
            .subscribe_reliable("Inspector.detached", None);
        tokio::spawn(async move {
            let mut state_sweep = tokio::time::interval(ROUTER_STATE_SWEEP_INTERVAL);
            state_sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    biased;
                    _ = state_sweep.tick() => {
                        router.sweep_transient_target_state().await;
                    }
                    event = attached_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        let Ok(attached) =
                            serde_json::from_value::<EventAttachedToTarget>(event.params)
                        else {
                            continue;
                        };
                        if attached.target_info.r#type == "page" {
                            router
                                .handle_attached(PendingPage {
                                    target_id: String::from(attached.target_info.target_id),
                                    session_id: String::from(attached.session_id),
                                    opener_target_id: attached
                                        .target_info
                                        .opener_id
                                        .map(String::from),
                                    target_url: Some(String::from(attached.target_info.url)),
                                })
                                .await;
                        }
                    }
                    event = detached_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        let Ok(detached) =
                            serde_json::from_value::<EventDetachedFromTarget>(event.params)
                        else {
                            continue;
                        };
                        router
                            .handle_top_level_session_loss(
                                None,
                                Some(String::from(detached.session_id)),
                                TopLevelTargetLoss::Detached,
                            )
                            .await;
                    }
                    event = destroyed_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        let Ok(destroyed) =
                            serde_json::from_value::<EventTargetDestroyed>(event.params)
                        else {
                            continue;
                        };
                        router
                            .handle_top_level_target_loss(
                                Some(String::from(destroyed.target_id)),
                                None,
                                TopLevelTargetLoss::Destroyed,
                            )
                            .await;
                    }
                    event = crashed_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        let Ok(crashed) =
                            serde_json::from_value::<EventTargetCrashed>(event.params)
                        else {
                            continue;
                        };
                        router
                            .handle_top_level_session_loss(
                                Some(String::from(crashed.target_id)),
                                None,
                                TopLevelTargetLoss::Crashed,
                            )
                            .await;
                    }
                    event = inspector_detached_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        if event.session_id.is_empty() {
                            continue;
                        }
                        router
                            .handle_top_level_session_loss(
                                None,
                                Some(event.session_id),
                                TopLevelTargetLoss::Detached,
                            )
                            .await;
                    }
                    barrier = barrier_rx.recv() => {
                        let Some(barrier) = barrier else {
                            break;
                        };
                        let _ = barrier.send(());
                    }
                }
            }
        })
    }
}

fn safe_download_guid_component(guid: &str) -> Option<&str> {
    let mut components = std::path::Path::new(guid).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(name)), None)
            if !guid.is_empty()
                && guid.len() <= 128
                && !guid.contains('/')
                && !guid.contains('\\')
                && name.to_str() == Some(guid) =>
        {
            Some(guid)
        }
        _ => None,
    }
}

fn is_chromium_download_guid_name(name: &str) -> bool {
    let guid = name
        .strip_suffix(".crdownload")
        .or_else(|| name.strip_suffix(".tmp"))
        .unwrap_or(name);
    if guid.len() != 36 {
        return false;
    }
    guid.bytes().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

fn remove_download_staging_file(path: &std::path::Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => {
            tracing::debug!(
                file = %path.display(),
                "removed browser download staging artifact"
            );
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            tracing::warn!(
                file = %path.display(),
                %error,
                "failed to remove browser download staging artifact; retained for retry"
            );
            false
        }
    }
}

fn abort_tab_record(record: &TabRecord) {
    record._inject_loop.abort();
    record._oopif_loop.abort();
    record._debug_loop.abort();
    record
        .oopif_managers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

/// Return a short diagnostic suffix without ever emitting a short CDP id in full.
///
/// Real Chromium target/session ids are long, so their final four characters
/// preserve useful correlation while avoiding persistence of the complete id.
/// The fallback is deliberately fixed for malformed or unusually short ids.
fn cdp_id_suffix(id: &str) -> String {
    let suffix = crate::tabs::last4(id);
    if suffix.chars().count() == id.chars().count() {
        "[redacted]".to_string()
    } else {
        suffix
    }
}

#[cfg(test)]
mod cdp_id_suffix_tests {
    use super::cdp_id_suffix;

    #[test]
    fn keeps_only_the_last_four_characters_of_a_long_id() {
        assert_eq!(cdp_id_suffix("0123456789abcdef"), "cdef");
    }

    #[test]
    fn redacts_short_ids_instead_of_logging_them_in_full() {
        assert_eq!(cdp_id_suffix("abcd"), "[redacted]");
        assert_eq!(cdp_id_suffix("x"), "[redacted]");
        assert_eq!(cdp_id_suffix(""), "[redacted]");
    }

    #[test]
    fn uses_character_boundaries_for_non_ascii_ids() {
        assert_eq!(cdp_id_suffix("target-一二三四五"), "二三四五");
        assert_eq!(cdp_id_suffix("一二三四"), "[redacted]");
    }
}

/// Choose the next active target without depending on `HashMap` iteration
/// order. The crashed target is filtered defensively even though callers
/// normally remove it before selecting a survivor.
fn deterministic_survivor<'a>(
    target_ids: impl IntoIterator<Item = &'a str>,
    crashed_target_id: &str,
) -> Option<String> {
    target_ids
        .into_iter()
        .filter(|target_id| *target_id != crashed_target_id)
        .min()
        .map(str::to_owned)
}

fn tab_handles(record: &TabRecord) -> TabHandles {
    TabHandles {
        target_id: record.target_id.clone(),
        session_id: record.session_id.clone(),
        injection: record.injection.clone(),
        main_frame_id: record.main_frame_id.clone(),
        oopif_managers: record.oopif_managers.clone(),
        ref_table: record.ref_table.clone(),
        debug: record.debug.clone(),
    }
}

/// RAII authority for Host initialization after the process has launched but
/// before all spawned runtime loops have been committed into `CdpHostRuntime`.
///
/// Tokio detaches a task when its JoinHandle is dropped. Cancellation of
/// `from_launched` therefore transfers every fixed per-Host handle together
/// with the exact process/profile cleanup authority into the existing bounded
/// cleanup relay. The relay aborts and joins the tasks before publishing exact
/// cleanup completion.
struct PendingCdpHostRuntime {
    cleanup: Arc<DurableProcessCleanup>,
    attach_loop: Option<tokio::task::JoinHandle<()>>,
    target_router_loop: Option<tokio::task::JoinHandle<()>>,
    download_loop: Option<tokio::task::JoinHandle<()>>,
    firewall_runtime: Option<FirewallLoopRuntime>,
    armed: bool,
}

impl PendingCdpHostRuntime {
    fn new(cleanup: Arc<DurableProcessCleanup>) -> Self {
        Self {
            cleanup,
            attach_loop: None,
            target_router_loop: None,
            download_loop: None,
            firewall_runtime: None,
            armed: true,
        }
    }

    fn commit(
        mut self,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        Option<tokio::task::JoinHandle<()>>,
        FirewallLoopRuntime,
    ) {
        self.armed = false;
        (
            self.attach_loop
                .take()
                .expect("published CDP Host owns an attach loop"),
            self.target_router_loop
                .take()
                .expect("published CDP Host owns a target-router loop"),
            self.download_loop.take(),
            self.firewall_runtime
                .take()
                .expect("published CDP Host owns a firewall runtime"),
        )
    }
}

impl Drop for PendingCdpHostRuntime {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut tasks = Vec::with_capacity(4);
        if let Some(handle) = self.attach_loop.take() {
            tasks.push(handle);
        }
        if let Some(handle) = self.target_router_loop.take() {
            tasks.push(handle);
        }
        if let Some(handle) = self.download_loop.take() {
            tasks.push(handle);
        }
        if let Some(runtime) = self.firewall_runtime.take()
            && let Some(handle) = runtime.into_pending_cleanup_handle()
        {
            tasks.push(handle);
        }
        for task in &tasks {
            task.abort();
        }
        self.cleanup.hand_off_with_runtime_tasks(tasks);
    }
}

/// Process/transport lifetime shared by all lanes on one managed host.
pub(crate) struct CdpHostRuntime {
    conn: Connection,
    /// Single exact process/profile cleanup authority. Explicit shutdown uses
    /// it synchronously; dropping the final Lane hands the same authority to a
    /// background worker, so product paths that return only a Lane cannot leak
    /// Chromium or its profile artifacts.
    cleanup: Arc<DurableProcessCleanup>,
    /// Read-only root Chromium pid captured before the child handle is moved.
    /// It is process-local telemetry only and never enters browser APIs,
    /// capability payloads, CDP routing, or profile metadata.
    root_process_id: Option<u32>,
    /// Start time captured from the exact managed-child ownership marker.
    /// Pairing it with the pid prevents telemetry from charging an unrelated
    /// process after operating-system pid reuse.
    root_process_started_at_epoch_seconds: u64,
    root_process_platform_start_key: u64,
    /// Watches the sticky transport-fatal signal and authoritatively retires
    /// the Chromium tree even when no caller issues another operation.
    fatal_supervisor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    attach_loop: tokio::task::JoinHandle<()>,
    target_router_loop: tokio::task::JoinHandle<()>,
    download_loop: Option<tokio::task::JoinHandle<()>>,
    /// Host-owned firewall cancellation, registered fixed worker tree, and
    /// watchdog. Explicit shutdown cancels and bounded-joins it; Drop aborts it.
    firewall_runtime: FirewallLoopRuntime,
    router: Arc<HostTargetRouter>,
    download_dir: Option<String>,
    firewall_config: crate::firewall::FirewallConfig,
    approved_domains: crate::firewall::ApprovedDomains,
    storage_state: Option<serde_json::Value>,
    headful: bool,
    display_available: bool,
    shutdown: AtomicBool,
    stopped: AtomicBool,
    shutdown_gate: AsyncMutex<()>,
}

impl CdpHostRuntime {
    pub(crate) async fn launch_in_mode(
        mut config: EngineConfig,
        requested_mode: BrowserHostLaunchMode,
        host_cleanup_lease: HostCleanupLease,
    ) -> Result<Arc<Self>, BrowserError> {
        config.headful = requested_mode.is_headful();
        let chrome_path = crate::acquire::resolve_chrome_path_with_source(
            &config.data_dir,
            config.bundled_dir.as_deref(),
            config.chrome_source,
        )
        .await?;
        let user_data_dir = crate::resolve_user_data_dir(&config);
        let launch_config = LaunchConfig {
            chrome_path,
            user_data_dir: user_data_dir.clone(),
            headful: config.headful,
        };
        // Downloads land in a per-exact-Host staging directory derived from
        // the trusted root process identity after launch; only a real task
        // workspace may serve as a Lane's final output fallback. `data_dir`
        // is never a download destination anymore.
        let download_staging_root =
            Some(crate::download::download_staging_root(&config.data_dir));
        let lane_fallback_download_dir = config
            .workspace_dir
            .as_deref()
            .map(crate::download::ensure_download_dir);
        let _launch_permit = crate::launch_semaphore()
            .acquire()
            .await
            .expect("browser launch semaphore is never closed");
        let display_available = crate::display::display_available();
        let effective_mode = if display_available {
            requested_mode
        } else {
            BrowserHostLaunchMode::Headless
        };
        let headful = effective_mode.is_headful();
        let cleanup_user_data_dir =
            config.ephemeral_profile.then_some(user_data_dir.clone());
        let launched = launch_chrome_with_cleanup_profile(
            &launch_config,
            effective_mode == BrowserHostLaunchMode::Headless,
            cleanup_user_data_dir,
            Some(host_cleanup_lease),
        )
        .await?;
        // `Launched` now carries the only profile-cleanup authority. Do not
        // pass `user_data_dir` separately: doing so would let a stable launch
        // be upgraded into whole-profile deletion during Host construction.
        Self::from_launched(
            launched,
            headful,
            display_available,
            download_staging_root,
            lane_fallback_download_dir,
            config.firewall,
            config.egress_approver,
            config.storage_state,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn from_launched(
        launched: Launched,
        headful: bool,
        display_available: bool,
        download_staging_root: Option<PathBuf>,
        lane_fallback_download_dir: Option<String>,
        firewall: crate::firewall::FirewallConfig,
        egress_approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
        storage_state: Option<serde_json::Value>,
        dns_resolver: Option<Arc<dyn crate::firewall::HostResolver>>,
    ) -> Result<Arc<Self>, BrowserError> {
        let (
            process,
            transport,
            ownership_token,
            launched_cleanup_user_data_dir,
            host_cleanup_lease,
        ) = launched.into_managed();
        let root_process_started_at_epoch_seconds =
            ownership_token.browser_start_time_epoch_seconds();
        let root_process_platform_start_key =
            ownership_token.browser_platform_start_key();
        let cleanup = Arc::new(DurableProcessCleanup::new(
            process,
            LaunchedProfileCleanupAuthority::new(launched_cleanup_user_data_dir),
            ownership_token.clone(),
            host_cleanup_lease,
        ));
        let mut pending_runtime = PendingCdpHostRuntime::new(Arc::clone(&cleanup));
        let root_process_id = cleanup.process_id();
        // Every Host stages downloads in its own exclusive directory named by
        // the exact root process identity. Without that identity there is no
        // cleanup-ownership proof, so downloads are denied outright below.
        let download_staging_dir = match (&download_staging_root, root_process_id) {
            (Some(staging_root), Some(pid)) => {
                crate::download::sweep_orphan_host_staging_dirs(staging_root);
                let staging_dir = staging_root.join(
                    crate::download::host_staging_dir_name(
                        pid,
                        root_process_platform_start_key,
                    ),
                );
                match std::fs::create_dir_all(&staging_dir) {
                    Ok(()) => Some(staging_dir),
                    Err(error) => {
                        tracing::warn!(
                            dir = %staging_dir.display(),
                            %error,
                            "exclusive download staging directory creation failed; downloads are disabled for this host"
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        let conn = match Connection::connect_launched(transport).await {
            Ok(conn) => conn,
            Err(error) => {
                return Err(map_transport_err(error));
            }
        };
        // F1：防火墙的可靠订阅必须在 attach loop 之前注册——handle_attached 的
        // Fetch.enable arming gate 依赖它；早于防火墙循环 spawn 的事件在 unbounded
        // 通道里缓存不丢。
        let firewall_subscriptions = FetchFirewallSubscriptions::subscribe(&conn);
        // Install the trusted task/Lane session authority before auto-attach.
        // Otherwise startup service workers can be released through the
        // historical unscoped double-registration window before the router
        // exists, permanently bypassing per-family admission.
        let router = match HostTargetRouter::try_new(
            conn.clone(),
            Some(Arc::clone(&cleanup)),
            download_staging_dir.clone(),
        ) {
            Ok(router) => router,
            Err(error) => {
                conn.shutdown().await;
                return Err(error);
            }
        };
        // From here on the download ledger (routes, task reservations, and
        // the exclusive staging directory) is retained by the exact process
        // cleanup authority and reconciled only after proven stop.
        cleanup.install_post_stop_reconcile(
            Arc::clone(&router.download_ledger) as Arc<dyn crate::launch::HostStopReconcile>
        );
        pending_runtime.target_router_loop = Some(router.spawn());
        pending_runtime.attach_loop = Some(conn.run_attach_loop());
        if let Err(error) = conn.enable_auto_attach().await {
            conn.shutdown().await;
            return Err(map_transport_err(error));
        }

        if let Some(ref staging_dir) = download_staging_dir {
            let handle = spawn_download_loop(conn.clone(), Some(router.clone()));
            pending_runtime.download_loop = Some(handle);
            let staging_path = staging_dir.to_string_lossy().into_owned();
            if let Err(error) = set_download_behavior_sandbox(&conn, &staging_path).await {
                conn.shutdown().await;
                return Err(error);
            }
        } else if let Err(error) = set_download_behavior_deny(&conn).await {
            // Without an exclusive staging identity Chromium must never fall
            // back to its default (user Downloads) directory.
            conn.shutdown().await;
            return Err(error);
        }

        let firewall_config = firewall.clone();
        let approved_domains = crate::firewall::ApprovedDomains::new();
        let dns_resolver = dns_resolver
            .unwrap_or_else(|| Arc::new(crate::firewall::TokioResolver::default()));
        pending_runtime.firewall_runtime = Some(spawn_fetch_firewall_loop(
            conn.clone(),
            firewall_subscriptions,
            firewall,
            egress_approver,
            approved_domains.clone(),
            dns_resolver,
            crate::firewall::DnsResolverCache::default(),
        ));
        if let Err(error) = enable_fetch_on_session(&conn, ROOT_SESSION).await {
            tracing::warn!(%error, "Fetch.enable on shared browser session failed");
        }

        let (attach_loop, target_router_loop, download_loop, firewall_runtime) =
            pending_runtime.commit();
        let runtime = Arc::new(Self {
            conn,
            cleanup,
            root_process_id,
            root_process_started_at_epoch_seconds,
            root_process_platform_start_key,
            fatal_supervisor: Mutex::new(None),
            attach_loop,
            target_router_loop,
            download_loop,
            firewall_runtime,
            router,
            download_dir: lane_fallback_download_dir,
            firewall_config,
            approved_domains,
            storage_state,
            headful,
            display_available,
            shutdown: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            shutdown_gate: AsyncMutex::new(()),
        });
        let mut fatal = runtime.conn.subscribe_fatal();
        let weak_runtime = Arc::downgrade(&runtime);
        let supervisor = tokio::spawn(async move {
            loop {
                if fatal.borrow().is_some() {
                    break;
                }
                if fatal.changed().await.is_err() {
                    return;
                }
            }
            let Some(runtime) = weak_runtime.upgrade() else {
                return;
            };
            tracing::error!(
                "browser transport terminated abnormally; retiring exact Chromium process tree"
            );
            if let Err(error) = runtime.shutdown().await {
                tracing::error!(%error, "synchronous transport-fatal cleanup did not converge; handing exact authority to cleanup relay");
                runtime.cleanup.hand_off();
                runtime.stopped.store(true, Ordering::Release);
            }
        });
        runtime
            .fatal_supervisor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(supervisor);
        Ok(runtime)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn process_id(&self) -> Option<u32> {
        if self.is_stopped() {
            None
        } else {
            self.root_process_id
        }
    }

    pub(crate) fn process_identity(&self) -> Option<(u32, u64, u64)> {
        self.process_id().map(|process_id| {
            (
                process_id,
                self.root_process_started_at_epoch_seconds,
                self.root_process_platform_start_key,
            )
        })
    }

    pub(crate) fn is_headful(&self) -> bool {
        self.headful && self.display_available
    }

    pub(crate) async fn task_lane_ids(&self, task_resource_key: &str) -> Vec<LaneId> {
        self.router.task_lane_ids(task_resource_key).await
    }

    pub(crate) async fn prepare_task_tab_limit_reconciliation(
        &self,
        task_resource_key: &str,
        max_task_tabs: usize,
    ) -> Result<TaskTabLimitReconcilePlan, BrowserError> {
        self.router
            .prepare_task_tab_limit_reconciliation(task_resource_key, max_task_tabs)
            .await
    }

    pub(crate) async fn task_tab_count(&self, task_resource_key: &str) -> usize {
        self.router.task_tab_count(task_resource_key).await
    }

    pub(crate) async fn shutdown(&self) -> Result<(), BrowserError> {
        let _shutdown = self.shutdown_gate.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        self.shutdown.store(true, Ordering::Release);
        self.target_router_loop.abort();
        self.firewall_runtime.shutdown().await;
        self.attach_loop.abort();
        if let Some(loop_handle) = &self.download_loop {
            loop_handle.abort();
        }
        use chromiumoxide::cdp::browser_protocol::browser::CloseParams;
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.conn.send::<CloseParams>(ROOT_SESSION, &CloseParams::default()),
        )
        .await;
        self.conn.shutdown().await;
        self.cleanup.finish().await?;
        self.router.finalize_downloads_after_host_stop();
        let residual_cleanup = self.router.retry_staging_cleanup();
        self.stopped.store(true, Ordering::Release);
        if residual_cleanup == 0 {
            Ok(())
        } else {
            Err(BrowserError::Other(format!(
                "browser stopped but {residual_cleanup} exact download staging artifact(s) remain cleanup-pending"
            )))
        }
    }
}

/// Profile-deletion authority carried by the indivisible [`Launched`] value.
///
/// Host constructors deliberately have no separate profile-path parameter:
/// stable launches (`None`) cannot be upgraded into whole-profile deletion,
/// and ephemeral launches retain exactly the path authorized at launch time.
struct LaunchedProfileCleanupAuthority(Option<PathBuf>);

impl LaunchedProfileCleanupAuthority {
    fn new(profile_dir: Option<PathBuf>) -> Self {
        Self(profile_dir)
    }

    fn into_profile_dir(self) -> Option<PathBuf> {
        self.0
    }
}

struct DurableProcessCleanup {
    /// `Some` means this object still owns exact process authority. `hand_off`
    /// takes the Arc out, so launch.rs/nomi-process-runtime really receives
    /// the final authority even if this wrapper remains reachable through a
    /// poisoned Host.
    process: std::sync::Mutex<
        Option<Arc<AsyncMutex<nomi_process_runtime::ManagedChildProcess>>>,
    >,
    handoff_ticket:
        tokio::sync::watch::Sender<Option<crate::launch::DroppedBrowserCleanupTicket>>,
    /// Serialize every explicit cleanup attempt. Host shutdown and poisoned
    /// target cleanup can race with one another; without this gate, one
    /// finisher can observe `process == None` before another finisher publishes
    /// sticky state `2`, then wait forever for a handoff ticket which a racing
    /// direct finisher made unnecessary.
    finish_gate: AsyncMutex<()>,
    state: AtomicU64,
    cleanup_user_data_dir: Option<PathBuf>,
    ownership_token: crate::profile::BrowserOwnershipToken,
    /// Opaque structural Host authority. Direct cleanup retains it here;
    /// `hand_off` moves it into the durable completion ticket so asynchronous
    /// cleanup debt cannot be used to mint another live Host.
    host_cleanup_lease: std::sync::Mutex<Option<HostCleanupLease>>,
    /// Post-stop reconciliation authority (the Host download ledger). Direct
    /// cleanup runs it after proving the exact process tree stopped;
    /// `hand_off` moves it into the relay job so a dropped Host cannot
    /// release retained download reservations before that proof exists.
    post_stop_reconcile:
        std::sync::Mutex<Option<Arc<dyn crate::launch::HostStopReconcile>>>,
    #[cfg(test)]
    test_hooks: Option<Arc<DurableProcessCleanupTestHooks>>,
}

#[cfg(test)]
#[derive(Default)]
struct DurableProcessCleanupTestHooks {
    finish_calls: AtomicUsize,
    finish_gate_entries: AtomicUsize,
    handoff_claimed: Option<Arc<std::sync::Barrier>>,
    handoff_release: Option<Arc<std::sync::Barrier>>,
}

impl DurableProcessCleanup {
    fn new(
        process: nomi_process_runtime::ManagedChildProcess,
        cleanup_authority: LaunchedProfileCleanupAuthority,
        ownership_token: crate::profile::BrowserOwnershipToken,
        host_cleanup_lease: Option<HostCleanupLease>,
    ) -> Self {
        let (handoff_ticket, _) = tokio::sync::watch::channel(None);
        Self {
            process: std::sync::Mutex::new(Some(Arc::new(AsyncMutex::new(process)))),
            handoff_ticket,
            finish_gate: AsyncMutex::new(()),
            state: AtomicU64::new(0),
            cleanup_user_data_dir: cleanup_authority.into_profile_dir(),
            ownership_token,
            host_cleanup_lease: std::sync::Mutex::new(host_cleanup_lease),
            post_stop_reconcile: std::sync::Mutex::new(None),
            #[cfg(test)]
            test_hooks: None,
        }
    }

    /// Installs the Host-scoped reconcile authority. Must run before the Host
    /// is published to callers so no hand-off can precede installation.
    fn install_post_stop_reconcile(
        &self,
        reconcile: Arc<dyn crate::launch::HostStopReconcile>,
    ) {
        self.post_stop_reconcile
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(reconcile);
    }

    fn process_id(&self) -> Option<u32> {
        let process = self
            .process
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned()?;
        process
            .try_lock()
            .ok()
            .and_then(|process| process.id())
    }

    fn hand_off(&self) {
        self.hand_off_with_runtime_tasks(Vec::new());
    }

    fn hand_off_with_runtime_tasks(&self, runtime_tasks: Vec<tokio::task::JoinHandle<()>>) {
        if self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            for task in runtime_tasks {
                task.abort();
            }
            return;
        }
        #[cfg(test)]
        if let Some(hooks) = self.test_hooks.as_ref()
            && let (Some(claimed), Some(release)) = (
                hooks.handoff_claimed.as_ref(),
                hooks.handoff_release.as_ref(),
            )
        {
            claimed.wait();
            release.wait();
        }
        let process = self
            .process
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(process) = process else {
            for task in runtime_tasks {
                task.abort();
            }
            return;
        };
        let host_cleanup_lease = self
            .host_cleanup_lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let post_stop_reconcile = self
            .post_stop_reconcile
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let ticket = crate::launch::hand_off_dropped_browser_cleanup_with_host_lease_and_tasks(
            process,
            self.ownership_token.clone(),
            self.cleanup_user_data_dir.clone(),
            host_cleanup_lease,
            runtime_tasks,
            crate::launch::PostStopReconcileCell::new(post_stop_reconcile),
        );
        self.handoff_ticket.send_replace(Some(ticket));
    }

    async fn finish(&self) -> Result<(), BrowserError> {
        #[cfg(test)]
        if let Some(hooks) = self.test_hooks.as_ref() {
            hooks.finish_calls.fetch_add(1, Ordering::AcqRel);
        }
        let _finish = self.finish_gate.lock().await;
        #[cfg(test)]
        if let Some(hooks) = self.test_hooks.as_ref() {
            hooks.finish_gate_entries.fetch_add(1, Ordering::AcqRel);
        }
        if self.state.load(Ordering::Acquire) == 2 {
            return Ok(());
        }
        let process = self
            .process
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned();
        let Some(process) = process else {
            // A racing direct finisher may have published sticky completion
            // and removed the process between the first state load and this
            // snapshot. Never wait for a handoff ticket in that completed
            // state: a handoff which lost the process race correctly has no
            // ticket to publish.
            if self.state.load(Ordering::Acquire) == 2 {
                return Ok(());
            }
            let mut ticket_rx = self.handoff_ticket.subscribe();
            let ticket = loop {
                if self.state.load(Ordering::Acquire) == 2 {
                    return Ok(());
                }
                if let Some(ticket) = ticket_rx.borrow().clone() {
                    break ticket;
                }
                ticket_rx.changed().await.map_err(|_| {
                    BrowserError::Other(
                        "browser process cleanup relay completion channel closed".into(),
                    )
                })?;
            };
            return match ticket.wait_or_retry().await {
                crate::launch::DroppedBrowserCleanupCompletion::Complete => {
                    self.state.store(2, Ordering::Release);
                    Ok(())
                }
                crate::launch::DroppedBrowserCleanupCompletion::RetryPending => {
                    Err(BrowserError::Other(
                        "browser process/profile cleanup remains pending with retained retry authority".into(),
                    ))
                }
            };
        };
        let result = {
            let mut process = process.lock().await;
            terminate_launched_process_tree_and_cleanup_profile(
                &mut process,
                &self.ownership_token,
                self.cleanup_user_data_dir.as_deref(),
            )
            .await
        };
        if result.is_ok() {
            self.state.store(2, Ordering::Release);
            self.process
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            self.host_cleanup_lease
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            // Exact process-tree stop (and profile artifact cleanup) is now
            // proven, so the retained download state may be reconciled and
            // its reservations released.
            let reconcile = self
                .post_stop_reconcile
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(reconcile) = reconcile
                && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    reconcile.reconcile_after_exact_host_stop();
                }))
                .is_err()
            {
                tracing::error!(
                    "post-stop host reconcile panicked during direct cleanup; retained download state was still released"
                );
            }
        }
        result
    }
}

impl Drop for DurableProcessCleanup {
    fn drop(&mut self) {
        if self.state.load(Ordering::Acquire) != 0 {
            return;
        }
        self.hand_off();
    }
}

impl Drop for CdpHostRuntime {
    fn drop(&mut self) {
        if let Some(supervisor) = self
            .fatal_supervisor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            supervisor.abort();
        }
        self.target_router_loop.abort();
        self.firewall_runtime.abort();
        self.attach_loop.abort();
        // Fence and cancel-request every tracked download; reservations stay
        // retained. Only the durable cleanup relay may release them, after it
        // proves the exact Chromium process tree stopped.
        self.router.poison_downloads_for_host_stop();
        self.router.retry_staging_cleanup();
        // The download loop mutates the retained ledger, so its JoinHandle is
        // settled by the relay before process termination and reconcile. The
        // bounded cleanup executor can still be retained by a queued target
        // authority; explicit handoff prevents that bounded cycle from
        // postponing whole-tree teardown after the Host itself is gone.
        let mut runtime_tasks = Vec::with_capacity(1);
        if let Some(handle) = self.download_loop.take() {
            handle.abort();
            runtime_tasks.push(handle);
        }
        self.cleanup.hand_off_with_runtime_tasks(runtime_tasks);
    }
}

#[derive(Clone)]
enum TargetCleanupJob {
    PendingPage(Arc<PendingCreatedPageCleanup>),
    Lane(Arc<LaneCleanupAuthority>),
    RouterTarget {
        router: Arc<HostTargetRouter>,
        lane_id: LaneId,
        target_id: String,
    },
    QuarantinedTarget {
        router: Arc<HostTargetRouter>,
        target_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetCleanupOutcome {
    Finished,
    EscalateHost,
}

impl TargetCleanupJob {
    async fn run(self) -> TargetCleanupOutcome {
        if let Self::PendingPage(cleanup) = &self
            && cleanup.target_id.is_none()
        {
            tracing::error!(
                pending_url = ?cleanup.pending_url,
                "pending create has no exact target identity; escalating instead of accepting a transient empty inventory"
            );
            return TargetCleanupOutcome::EscalateHost;
        }
        let conn = match &self {
            Self::PendingPage(cleanup) => cleanup.conn.clone(),
            Self::Lane(cleanup) => cleanup.conn.clone(),
            Self::RouterTarget { router, .. } => router.conn.clone(),
            Self::QuarantinedTarget { router, .. } => router.conn.clone(),
        };
        match tokio::time::timeout(TARGET_CLEANUP_JOB_BUDGET, async {
            match self {
                Self::PendingPage(cleanup) => cleanup.finish().await,
                Self::Lane(cleanup) => cleanup.finish().await,
                Self::RouterTarget {
                    router,
                    lane_id,
                    target_id,
                } => {
                    router
                        .retry_cleanup_only_target(Some(lane_id), target_id)
                        .await;
                }
                Self::QuarantinedTarget { router, target_id } => {
                    router
                        .retry_cleanup_only_target(None, target_id)
                        .await;
                }
            }
        })
        .await
        {
            Ok(()) if conn.registry().is_connection_closed() => {
                // A dead CDP transport is not proof that Chromium exited.
                TargetCleanupOutcome::EscalateHost
            }
            Ok(()) => TargetCleanupOutcome::Finished,
            Err(_) => TargetCleanupOutcome::EscalateHost,
        }
    }

    /// State `3` means that exact-target cleanup has been subsumed by an
    /// authoritative whole-Host teardown. It is terminal for this individual
    /// worker (no duplicate target job may be submitted), while the executor
    /// retains the first overflow intent until process exit is proven.
    fn mark_host_escalated(&self) {
        match self {
            Self::PendingPage(cleanup) => {
                let _ = cleanup.state.compare_exchange(
                    1,
                    3,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            Self::Lane(cleanup) => {
                cleanup.lane_closing.store(true, Ordering::Release);
                let _ = cleanup.state.compare_exchange(
                    1,
                    3,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            Self::RouterTarget { .. } | Self::QuarantinedTarget { .. } => {}
        }
    }

    fn mark_host_cleanup_finished(&self) {
        match self {
            Self::PendingPage(cleanup) => cleanup.mark_finished(),
            Self::Lane(cleanup) => cleanup.mark_finished(),
            Self::RouterTarget { .. } | Self::QuarantinedTarget { .. } => {}
        }
    }
}

/// One bounded cleanup executor per Host (and one per standalone backend).
///
/// Exactly one target-cleanup future runs at a time and the mailbox has a hard
/// capacity. If it saturates, or if its sole worker disappears, the executor
/// becomes permanently poisoned and escalates to whole-browser process
/// teardown. That preserves a finite memory/CDP bound without dropping cleanup
/// authority for the target which did not fit in the mailbox.
struct TargetCleanupExecutor {
    sender: tokio::sync::mpsc::Sender<TargetCleanupJob>,
    conn: Connection,
    process_cleanup: Option<Arc<DurableProcessCleanup>>,
    poisoned: Arc<AtomicBool>,
    escalation: Arc<tokio::sync::Notify>,
    overflow_job: Arc<std::sync::Mutex<Option<TargetCleanupJob>>>,
    #[allow(dead_code)]
    worker_count: Arc<AtomicUsize>,
    #[allow(dead_code)]
    active_jobs: Arc<AtomicUsize>,
    #[allow(dead_code)]
    max_active_jobs: Arc<AtomicUsize>,
    #[allow(dead_code)]
    queued_jobs: Arc<AtomicUsize>,
    #[allow(dead_code)]
    max_queued_jobs: Arc<AtomicUsize>,
}

impl TargetCleanupExecutor {
    fn new(
        conn: Connection,
        process_cleanup: Option<Arc<DurableProcessCleanup>>,
    ) -> Result<Arc<Self>, BrowserError> {
        #[cfg(test)]
        if TARGET_CLEANUP_EXECUTOR_START_FAILURE.with(std::cell::Cell::take) {
            return Err(BrowserError::Other(
                "injected target cleanup executor start failure".into(),
            ));
        }
        let (sender, mut receiver) =
            tokio::sync::mpsc::channel::<TargetCleanupJob>(TARGET_CLEANUP_QUEUE_CAPACITY);
        let poisoned = Arc::new(AtomicBool::new(false));
        let escalation = Arc::new(tokio::sync::Notify::new());
        let overflow_job = Arc::new(std::sync::Mutex::new(None));
        let worker_count = Arc::new(AtomicUsize::new(0));
        let active_jobs = Arc::new(AtomicUsize::new(0));
        let max_active_jobs = Arc::new(AtomicUsize::new(0));
        let queued_jobs = Arc::new(AtomicUsize::new(0));
        let max_queued_jobs = Arc::new(AtomicUsize::new(0));

        let conn_for_thread = conn.clone();
        let process_cleanup_for_thread = process_cleanup.clone();
        let poisoned_for_thread = Arc::clone(&poisoned);
        let escalation_for_thread = Arc::clone(&escalation);
        let overflow_job_for_thread = Arc::clone(&overflow_job);
        let worker_count_for_thread = Arc::clone(&worker_count);
        let active_jobs_for_thread = Arc::clone(&active_jobs);
        let max_active_jobs_for_thread = Arc::clone(&max_active_jobs);
        let queued_jobs_for_thread = Arc::clone(&queued_jobs);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("nomi-target-cleanup".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        worker_count_for_thread.fetch_add(1, Ordering::AcqRel);
                        runtime
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        tracing::error!(
                            %error,
                            "target cleanup executor runtime creation failed"
                        );
                        return;
                    }
                };
                let _exit_guard = TargetCleanupWorkerExitGuard {
                    poisoned: Arc::clone(&poisoned_for_thread),
                    process_cleanup: process_cleanup_for_thread.clone(),
                };
                let _ = ready_tx.send(Ok(()));
                runtime.block_on(async move {
                    loop {
                        if poisoned_for_thread.load(Ordering::Acquire) {
                            finish_poisoned_target_cleanup_host(
                                &conn_for_thread,
                                process_cleanup_for_thread.as_ref(),
                                None,
                                &mut receiver,
                                &overflow_job_for_thread,
                                &queued_jobs_for_thread,
                            )
                            .await;
                            break;
                        }
                        let job = tokio::select! {
                            biased;
                            _ = escalation_for_thread.notified() => {
                                finish_poisoned_target_cleanup_host(
                                    &conn_for_thread,
                                    process_cleanup_for_thread.as_ref(),
                                    None,
                                    &mut receiver,
                                    &overflow_job_for_thread,
                                    &queued_jobs_for_thread,
                                )
                                .await;
                                break;
                            }
                            job = receiver.recv() => match job {
                                Some(job) => job,
                                None => break,
                            },
                        };
                        queued_jobs_for_thread.fetch_sub(1, Ordering::AcqRel);
                        let active = active_jobs_for_thread.fetch_add(1, Ordering::AcqRel) + 1;
                        max_active_jobs_for_thread.fetch_max(active, Ordering::AcqRel);
                        let active_job = job.clone();
                        let outcome = tokio::select! {
                            biased;
                            _ = escalation_for_thread.notified() => {
                                TargetCleanupOutcome::EscalateHost
                            },
                            outcome = job.run() => outcome,
                        };
                        active_jobs_for_thread.fetch_sub(1, Ordering::AcqRel);
                        if outcome == TargetCleanupOutcome::EscalateHost
                            || poisoned_for_thread.load(Ordering::Acquire)
                        {
                            poisoned_for_thread.store(true, Ordering::Release);
                            finish_poisoned_target_cleanup_host(
                                &conn_for_thread,
                                process_cleanup_for_thread.as_ref(),
                                Some(active_job),
                                &mut receiver,
                                &overflow_job_for_thread,
                                &queued_jobs_for_thread,
                            )
                            .await;
                            break;
                        }
                    }
                });
                worker_count_for_thread.fetch_sub(1, Ordering::AcqRel);
            })
            .map_err(|error| {
                BrowserError::Other(format!(
                    "could not create the bounded target cleanup executor: {error}"
                ))
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(BrowserError::Other(format!(
                    "could not initialize the bounded target cleanup executor: {error}"
                )));
            }
            Err(error) => {
                return Err(BrowserError::Other(format!(
                    "target cleanup executor exited before initialization: {error}"
                )));
            }
        }
        Ok(Arc::new(Self {
            sender,
            conn,
            process_cleanup,
            poisoned,
            escalation,
            overflow_job,
            worker_count,
            active_jobs,
            max_active_jobs,
            queued_jobs,
            max_queued_jobs,
        }))
    }

    /// A closed CDP transport proves only that per-target commands are no
    /// longer possible. Standalone Lane structural capacity remains charged
    /// until the containing Host's exact process/profile authority proves
    /// completion. Test-only routers have no process authority and treat the
    /// closed connection as their terminal boundary.
    async fn await_host_cleanup_proof(&self) {
        let Some(cleanup) = self.process_cleanup.as_ref() else {
            return;
        };
        loop {
            match cleanup.finish().await {
                Ok(()) => return,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Lane cleanup is retaining structural capacity while Host cleanup remains pending"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    fn ensure_accepting(&self) -> Result<(), BrowserError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(BrowserError::SessionLost { recoverable: false });
        }
        if self.conn.registry().is_connection_closed() {
            self.poison(None, true);
            return Err(BrowserError::SessionLost { recoverable: false });
        }
        Ok(())
    }

    fn submit(&self, job: TargetCleanupJob) {
        if self.poisoned.load(Ordering::Acquire) {
            job.mark_host_escalated();
            if let Some(cleanup) = self.process_cleanup.as_ref() {
                cleanup.hand_off();
            }
            return;
        }
        if self.conn.registry().is_connection_closed() {
            self.poison(Some(job), true);
            return;
        }

        let queued = self.queued_jobs.fetch_add(1, Ordering::AcqRel) + 1;
        match self.sender.try_send(job) {
            Ok(()) => {
                self.max_queued_jobs.fetch_max(queued, Ordering::AcqRel);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(job)) => {
                self.queued_jobs.fetch_sub(1, Ordering::AcqRel);
                self.poison(Some(job), false);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(job)) => {
                self.queued_jobs.fetch_sub(1, Ordering::AcqRel);
                self.poison(Some(job), true);
            }
        }
    }

    fn poison(&self, job: Option<TargetCleanupJob>, worker_unavailable: bool) {
        if let Some(job) = job.as_ref() {
            job.mark_host_escalated();
        }
        let first = !self.poisoned.swap(true, Ordering::AcqRel);
        if first {
            if let Some(job) = job {
                *self
                    .overflow_job
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(job);
            }
            self.escalation.notify_one();
        }
        if worker_unavailable {
            if let Some(cleanup) = self.process_cleanup.as_ref() {
                // The worker cannot perform Browser.close/Connection::shutdown;
                // exact process-tree authority is the fail-closed fallback.
                cleanup.hand_off();
            }
        }
    }

    #[cfg(test)]
    fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn worker_count(&self) -> usize {
        self.worker_count.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn max_active_jobs(&self) -> usize {
        self.max_active_jobs.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn max_queued_jobs(&self) -> usize {
        self.max_queued_jobs.load(Ordering::Acquire)
    }
}

/// Unexpected worker exit cannot strand a live Chromium process. The same
/// exact process/profile authority is deduplicated by DurableProcessCleanup.
struct TargetCleanupWorkerExitGuard {
    poisoned: Arc<AtomicBool>,
    process_cleanup: Option<Arc<DurableProcessCleanup>>,
}

impl Drop for TargetCleanupWorkerExitGuard {
    fn drop(&mut self) {
        self.poisoned.store(true, Ordering::Release);
        if let Some(cleanup) = self.process_cleanup.as_ref() {
            cleanup.hand_off();
        }
    }
}

async fn finish_poisoned_target_cleanup_host(
    conn: &Connection,
    process_cleanup: Option<&Arc<DurableProcessCleanup>>,
    active_job: Option<TargetCleanupJob>,
    receiver: &mut tokio::sync::mpsc::Receiver<TargetCleanupJob>,
    overflow_job: &std::sync::Mutex<Option<TargetCleanupJob>>,
    queued_jobs: &AtomicUsize,
) {
    let mut subsumed = Vec::with_capacity(TARGET_CLEANUP_QUEUE_CAPACITY + 2);
    if let Some(job) = active_job {
        job.mark_host_escalated();
        subsumed.push(job);
    }
    if let Some(job) = overflow_job
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        job.mark_host_escalated();
        subsumed.push(job);
    }
    while let Ok(job) = receiver.try_recv() {
        queued_jobs.fetch_sub(1, Ordering::AcqRel);
        job.mark_host_escalated();
        subsumed.push(job);
    }

    if !conn.registry().is_connection_closed() {
        use chromiumoxide::cdp::browser_protocol::browser::CloseParams;
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            conn.send::<CloseParams>(ROOT_SESSION, &CloseParams::default()),
        )
        .await;
    }
    conn.shutdown().await;

    let process_exit_proven = if let Some(cleanup) = process_cleanup {
        let mut proven = false;
        for attempt in 1..=TARGET_PROCESS_CLEANUP_ATTEMPTS {
            match tokio::time::timeout(
                TARGET_PROCESS_CLEANUP_ATTEMPT_BUDGET,
                cleanup.finish(),
            )
            .await
            {
                Ok(Ok(())) => {
                    proven = true;
                    break;
                }
                Ok(Err(error)) => {
                    tracing::error!(
                        %error,
                        attempt,
                        "bounded target cleanup is retrying authoritative Host teardown"
                    );
                }
                Err(_) => tracing::error!(
                    attempt,
                    "authoritative Host teardown exceeded its bounded attempt budget"
                ),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !proven {
            // This is an authority transfer, not abandonment: launch.rs hands
            // the exact process/token/profile bundle to its process-local
            // relay and leaves the marker for startup quarantine if needed.
            cleanup.hand_off();
            // The subsumed target/Lane jobs below still own structural task
            // reservations. Keep this fixed Host cleanup worker and those jobs
            // alive until the durable ticket proves process/profile absence;
            // permanent cleanup debt must remain charged, not reopen capacity.
            loop {
                match cleanup.finish().await {
                    Ok(()) => {
                        proven = true;
                        break;
                    }
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "poisoned Host cleanup debt still retains structural capacity"
                        );
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        }
        proven
    } else {
        // Test-only routers have no Chromium process authority; the closed
        // Connection is their terminal lifecycle boundary.
        true
    };
    if process_exit_proven {
        for job in subsumed {
            job.mark_host_cleanup_finished();
        }
    }
}

#[cfg(test)]
thread_local! {
    static TARGET_CLEANUP_EXECUTOR_START_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Exact cleanup authority for a target whose create command has completed
/// (or whose nonce-correlated attach was observed) but which has not yet been
/// transferred into a registered Lane.
struct PendingCreatedPageCleanup {
    conn: Connection,
    executor: Arc<TargetCleanupExecutor>,
    router: Option<Arc<HostTargetRouter>>,
    target_id: Option<String>,
    session_id: Option<String>,
    pending_url: Option<String>,
    /// Retained until exact close/absence proof (or authoritative Host
    /// teardown), so cleanup-pending targets continue consuming task quota.
    _task_tab_reservation: Option<Arc<dyn TaskTabReservation>>,
    /// Retains the structural Lane slot from admission until this pending
    /// target is either transferred into LaneCleanupAuthority or exact cleanup
    /// (including a delegated Host teardown) completes.
    _lane_resource_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    state: AtomicU64,
}

impl PendingCreatedPageCleanup {
    fn new(
        conn: Connection,
        executor: Arc<TargetCleanupExecutor>,
        router: Option<Arc<HostTargetRouter>>,
        target_id: Option<String>,
        session_id: Option<String>,
        pending_url: Option<String>,
        task_tab_reservation: Option<Arc<dyn TaskTabReservation>>,
        lane_resource_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            conn,
            executor,
            router,
            target_id,
            session_id,
            pending_url,
            _task_tab_reservation: task_tab_reservation,
            _lane_resource_authority: lane_resource_authority,
            state: AtomicU64::new(0),
        })
    }

    fn hand_off(self: &Arc<Self>) {
        if self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        self.executor
            .submit(TargetCleanupJob::PendingPage(Arc::clone(self)));
    }

    fn mark_finished(&self) {
        self.state.store(2, Ordering::Release);
        if let Some(authority) = &self._lane_resource_authority {
            authority.release_lane();
        }
    }

    async fn finish(self: Arc<Self>) {
        let target_id = match self.target_id.clone() {
            Some(target_id) => target_id,
            None => {
                // TargetCleanupJob::run classifies this as immediate Host
                // escalation. Never manufacture an absence proof here.
                return;
            }
        };
        let exact_absence_proven = loop {
            match close_target_or_confirm_absent(&self.conn, &target_id).await {
                Ok(()) => break true,
                Err(error) if self.conn.registry().is_connection_closed() => {
                    tracing::debug!(
                        target_id_suffix = %cdp_id_suffix(&target_id),
                        %error,
                        "pending-page cleanup delegated to closed Host transport"
                    );
                    break false;
                }
                Err(error) => {
                    tracing::warn!(
                        target_id_suffix = %cdp_id_suffix(&target_id),
                        %error,
                        "pending-page cleanup is retrying exact target close"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        };
        if exact_absence_proven && let Some(router) = self.router.as_ref() {
            router
                .scrub_unowned_pending_target(
                    &target_id,
                    self.session_id.as_deref(),
                    self._task_tab_reservation.as_ref(),
                )
                .await;
        }
        if !exact_absence_proven {
            self.executor.await_host_cleanup_proof().await;
        }
        self.mark_finished();
    }
}

struct PendingCreatedPage {
    target_id: String,
    session_id: String,
    cleanup: Arc<PendingCreatedPageCleanup>,
    task_tab_reservation: Option<Arc<dyn TaskTabReservation>>,
    transferred: bool,
}

impl PendingCreatedPage {
    fn transfer_to_lane(mut self) -> (String, String) {
        self.transferred = true;
        (self.target_id.clone(), self.session_id.clone())
    }
}

impl Drop for PendingCreatedPage {
    fn drop(&mut self) {
        if !self.transferred {
            self.cleanup.hand_off();
        }
    }
}

struct PendingPageCreateWaitGuard {
    cancellation: CancellationToken,
    completed: bool,
}

impl PendingPageCreateWaitGuard {
    fn new() -> Self {
        Self {
            cancellation: CancellationToken::new(),
            completed: false,
        }
    }

    fn token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for PendingPageCreateWaitGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
        }
    }
}

async fn create_pending_page_session_owned(
    conn: Connection,
    executor: Arc<TargetCleanupExecutor>,
    router: Option<Arc<HostTargetRouter>>,
    background: bool,
    task_tab_reservation_scope: Option<TaskTabReservationScope>,
) -> Result<PendingCreatedPage, BrowserError> {
    executor.ensure_accepting()?;
    let mut wait_guard = PendingPageCreateWaitGuard::new();
    let caller_cancelled = wait_guard.token();
    let result = tokio::spawn(async move {
        create_pending_lane_page_session(
            conn,
            executor,
            router,
            background,
            caller_cancelled,
            task_tab_reservation_scope,
        )
        .await
    })
    .await
    .map_err(|error| {
        BrowserError::Other(format!("owned browser page launch task failed: {error}"))
    })?;
    wait_guard.complete();
    result
}

/// Shared final-Drop authority for a Lane which has not completed explicit
/// shutdown. The pending launch guard and the returned backend share this
/// object, so cancellation both before and after `from_host` returns is
/// covered without letting two cleanup workers race.
struct LaneCleanupAuthority {
    conn: Connection,
    executor: Arc<TargetCleanupExecutor>,
    router: Arc<HostTargetRouter>,
    lane_id: LaneId,
    registration_id: AtomicU64,
    lane_closing: Arc<AtomicBool>,
    tabs: Arc<AsyncMutex<HashMap<String, TabRecord>>>,
    target_id: String,
    session_id: String,
    frame_id: String,
    initial_task_tab_reservation: Option<std::sync::Weak<dyn TaskTabReservation>>,
    /// Structural Lane authority is retained through Drop handoff and cleanup
    /// retries. Releasing it before exact target absence would let repeated
    /// open/drop cycles evade the per-task Lane cap.
    lane_resource_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    state: AtomicU64,
}

impl LaneCleanupAuthority {
    fn new(
        conn: Connection,
        executor: Arc<TargetCleanupExecutor>,
        router: Arc<HostTargetRouter>,
        lane_id: LaneId,
        lane_closing: Arc<AtomicBool>,
        tabs: Arc<AsyncMutex<HashMap<String, TabRecord>>>,
        target_id: String,
        session_id: String,
        frame_id: String,
        initial_task_tab_reservation: Option<&Arc<dyn TaskTabReservation>>,
        lane_resource_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            conn,
            executor,
            router,
            lane_id,
            registration_id: AtomicU64::new(0),
            lane_closing,
            tabs,
            target_id,
            session_id,
            frame_id,
            initial_task_tab_reservation: initial_task_tab_reservation.map(Arc::downgrade),
            lane_resource_authority,
            state: AtomicU64::new(0),
        })
    }

    fn set_registration(&self, registration_id: LaneRegistrationId) {
        let previous = self
            .registration_id
            .compare_exchange(
                0,
                registration_id.0,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .expect("a pending Lane is registered at most once");
        debug_assert_eq!(previous, 0);
    }

    fn registration(&self) -> Option<LaneRegistrationId> {
        match self.registration_id.load(Ordering::Acquire) {
            0 => None,
            registration_id => Some(LaneRegistrationId(registration_id)),
        }
    }

    fn mark_finished(&self) {
        self.state.store(2, Ordering::Release);
        if let Some(authority) = &self.lane_resource_authority {
            authority.release_lane();
        }
    }

    fn hand_off(self: &Arc<Self>) {
        if self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        self.lane_closing.store(true, Ordering::Release);
        self.executor
            .submit(TargetCleanupJob::Lane(Arc::clone(self)));
    }

    async fn finish(self: Arc<Self>) {
        let registration = self.registration();
        let expected_initial_reservation = self
            .initial_task_tab_reservation
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        let is_current = match registration {
            Some(registration_id) => {
                self.router
                    .is_current_registration(&self.lane_id, registration_id)
                    .await
            }
            None => false,
        };
        let mut target_ids = vec![self.target_id.clone()];
        // The tab registry is generation-local even if a replacement Lane
        // with the same public id has already been registered. Always retain
        // exact cleanup authority for those old-generation targets.
        target_ids.extend(self.tabs.lock().await.keys().cloned());
        if is_current {
            target_ids.extend(self.router.owned_targets(&self.lane_id).await);
        }
        target_ids.sort();
        target_ids.dedup();

        let mut initial_target_absence_proven = false;
        for target_id in target_ids {
            loop {
                match close_target_or_confirm_absent(&self.conn, &target_id).await {
                    Ok(()) => {
                        if target_id == self.target_id {
                            initial_target_absence_proven = true;
                        }
                        break;
                    }
                    Err(error) if self.conn.registry().is_connection_closed() => {
                        tracing::debug!(
                            lane_id = %self.lane_id,
                            target_id_suffix = %cdp_id_suffix(&target_id),
                            %error,
                            "pending-Lane target cleanup delegated to closed Host transport"
                        );
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(
                            lane_id = %self.lane_id,
                            target_id_suffix = %cdp_id_suffix(&target_id),
                            %error,
                            "pending-Lane cleanup is retrying exact target close"
                        );
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        }

        let unregistered = match registration {
            Some(registration_id) => {
                self.router
                    .unregister_lane_if_current(&self.lane_id, registration_id)
                    .await
            }
            None => false,
        };
        // F20: the retired finalizer must run whenever this Lane was ever
        // registered — not only when THIS call performed the unregister. An
        // explicit shutdown_lane may already have unregistered (moving owned
        // targets into retired tombstones) and then failed inside
        // finalize_retired_lane; this Drop/hand_off path is the last cleanup
        // authority for those still-live retired targets. finalize is
        // idempotent and cheap when nothing is retired. A never-registered
        // Lane has no retired inventory and skips it entirely.
        if registration.is_some() && !self.conn.registry().is_connection_closed() {
            loop {
                match self.router.finalize_retired_lane(&self.lane_id).await {
                    Ok(()) => break,
                    Err(_error) if self.conn.registry().is_connection_closed() => break,
                    Err(error) => {
                        tracing::warn!(
                            lane_id = %self.lane_id,
                            unregistered_by_this_call = unregistered,
                            %error,
                            "pending-Lane retired finalizer is retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        }

        let records = {
            let mut tabs = self.tabs.lock().await;
            std::mem::take(&mut *tabs)
        };
        for record in records.values() {
            abort_tab_record(record);
        }
        self.router
            .scrub_cancelled_lane_target(
                &self.lane_id,
                &self.target_id,
                &self.session_id,
                &self.frame_id,
                initial_target_absence_proven,
                expected_initial_reservation.as_ref(),
            )
            .await;
        if self.conn.registry().is_connection_closed() {
            self.executor.await_host_cleanup_proof().await;
        }
        self.mark_finished();
    }
}

struct PendingLaneLaunchGuard {
    cleanup: Arc<LaneCleanupAuthority>,
    committed: bool,
}

impl PendingLaneLaunchGuard {
    fn new(cleanup: Arc<LaneCleanupAuthority>) -> Self {
        Self {
            cleanup,
            committed: false,
        }
    }

    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for PendingLaneLaunchGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.cleanup.hand_off();
        }
    }
}

/// P0 浏览器后端：自建 transport 发裸 CDP 命令（无 chromiumoxide 高层）。
///
/// **P2 D1 结构改造（DESIGN §13 裁决⑥）**：原先「单 tab 的 per-tab 字段直挂」改为 tab 注册表 +
/// active_target 指针。per-tab 状态（page session / 注入管线 / OOPIF 表 / ref 表 / 主帧 id）下放进
/// [`TabRecord`]，存进 `tabs: HashMap<targetId, TabRecord>`；`active_target` 指向当前作用的 tab。
/// observe/act/navigate 经 [`Self::active_tab_handles`] 短暂锁 `tabs`+`active_target` 克隆出句柄、
/// **立即释放 `tabs` 锁**后用克隆句柄操作（不跨 await 持 `tabs` 锁——见 [`crate::tabs`] 模块级锁设计）。
///
/// **单 tab 场景**（D1 恒态；多 tab 填充是 D3）：`tabs` 恒只 1 项、`active_target` 指向它——行为与
/// 改造前完全一致。
pub struct CdpBackend {
    conn: Connection,
    /// Shared process/transport owner. A lane never owns the Chromium child.
    host: Option<Arc<CdpHostRuntime>>,
    /// Unit-test seam for exercising Lane retirement without constructing a
    /// real managed Chromium process. Production always routes through `host`.
    #[cfg(test)]
    test_router: Option<Arc<HostTargetRouter>>,
    lane_id: LaneId,
    task_tab_reservation_scope: Option<TaskTabReservationScope>,
    task_download_reservation_scope: Option<TaskDownloadReservationScope>,
    /// Opaque authority shared by every Lane/Host of the same trusted task.
    /// Only temporary act subscriptions charge it; Host router/firewall/
    /// download subscribers remain Host-owned.
    reliable_event_task_budget: Arc<ReliableEventTaskBudget>,
    /// Final-Drop cleanup fallback for a shared Host Lane. This remains armed
    /// across the `from_host` -> coordinator publication gap.
    lane_cleanup: Option<Arc<LaneCleanupAuthority>>,
    /// Bounded target cleanup executor shared by every target in this backend.
    cleanup_executor: Arc<TargetCleanupExecutor>,
    /// Lane closure is two-phase for cancellation safety: `shutdown_lane`
    /// sets `lane_closing` immediately to fence new work, but `lane_closed`
    /// is published only after every target/session cleanup step completes.
    /// If the close future is cancelled midway, the next authoritative retry
    /// can continue instead of returning a false idempotent success.
    lane_closing: Arc<AtomicBool>,
    lane_closed: Arc<AtomicBool>,
    /// The active target-drain phase completed and retirement is committed.
    ///
    /// This is published immediately before the idempotent router unregister
    /// call. A cancelled unregister/finalize attempt therefore retries the
    /// retired phase directly instead of consulting a Lane route that may
    /// already have been removed.
    lane_retired: AtomicBool,
    lane_shutdown_gate: AsyncMutex<()>,
    lane_close_confirmed: AsyncMutex<HashSet<String>>,
    lane_cancel: CancellationToken,
    /// **tab 注册表**：targetId → [`TabRecord`]（吸收原 per-tab 字段）。短暂锁、克隆句柄、立即释放
    /// （绝不跨 await 持有；见 [`crate::tabs`]）。**D3**：`Arc` 包裹——tab 发现循环（`'static` 后台任务）
    /// 持一份克隆，发现新顶层 page 时锁它插入新 [`TabRecord`]（与 observe/act 的短临界区共存，互不跨
    /// await 持锁）。
    tabs: Arc<AsyncMutex<HashMap<String, TabRecord>>>,
    /// **active_target 指针**：当前 observe/act/navigate 默认作用的 tab 的 targetId（DESIGN §13）。
    /// 指向不存在的 tab → [`Self::active_tab_handles`] 返 `BrowserError`（绝不 panic）。**D3**：`Arc`
    /// 包裹——switch/close 改它做逻辑指针切换；发现循环**不**改它（新 tab 不抢焦点）。
    active_target: Arc<AsyncMutex<String>>,
    /// **active_frame 指针**（D4 switch_frame，DESIGN §13）：`Some((session_id, frame_id))` = 已 switch_frame
    /// 切入某 iframe（**页面级动作**默认作用于它）；`None`（默认）= 主帧/顶层（页面级动作作用主文档）。
    /// switch_frame 改它；切 tab 后旧指针对不上当前 active tab → [`Self::active_page_frame`] 退主帧。
    /// **ref-based 动作不受它影响**（按 ref 所属帧路由，本就跨帧）。`Arc`：与其它共享态一致，短临界区锁。
    active_frame: Arc<AsyncMutex<Option<(String, String)>>>,
    /// **act 的 objectGroup 序号源**（C1，DESIGN §7）：每次 `act` `fetch_add(1)` 取一个唯一 `seq`，
    /// 拼成本动作的 objectGroup `act-<seq>`（[`crate::actionability::act_object_group`]）。唯一性保
    /// 证并发/连续动作的句柄组互不串味、各自 `releaseObjectGroup` 不误伤他组。`Relaxed` 足够——只
    /// 需单调唯一，不依赖与其它内存操作的顺序。**保留在 backend（全局即可，非 per-tab）**。
    act_seq: std::sync::atomic::AtomicU64,
    /// Standalone backend's single exact process/profile cleanup authority.
    /// Shared-host lanes leave this empty because [`CdpHostRuntime`] owns the
    /// same durable authority. Drop hands cleanup to a retrying worker.
    _process: Option<Arc<DurableProcessCleanup>>,
    /// attach 处理循环句柄——保活，让 flatten 自动附着的子 session 持续被登记。
    _attach_loop: Option<tokio::task::JoinHandle<()>>,
    /// **tab 发现后台循环句柄**（D3）——保活。订阅新顶层 page 的 `Target.attachedToTarget`，arm 成
    /// [`TabRecord`] 入 `tabs`（不抢焦点）。backend Drop 即连带 abort（连接随之关闭，循环也会自然退出）。
    _tab_discovery_loop: Option<tokio::task::JoinHandle<()>>,
    /// **下载事件后台循环句柄**（E4）——保活。仅当 `setDownloadBehavior` 沙箱已挂（`download_dir`
    /// 为 `Some`）时存在。订阅 `Browser.downloadProgress`，对完成（`state=="completed"`）的下载在其
    /// `filePath` 上打 Win MOTW（`Zone.Identifier` ADS）。mac/linux 为空实现。**绝不**自动打开文件。
    /// backend Drop 即连带 abort。
    _download_loop: Option<tokio::task::JoinHandle<()>>,
    /// **隔离下载目录的绝对路径**（E4 沙箱落点 / F-actions `download`/`save_as_pdf` 落点）。
    /// = [`crate::download::ensure_download_dir`] 的产物（`<per-pet workspace 或 data_dir>/downloads`，
    /// **绝不**用户真实 Downloads）。`Some` 当且仅当下载沙箱已挂（与 `_download_loop` 同生）。
    /// `download`（注入 `<a download>` 触发）的产物落这里、`save_as_pdf`（`Page.printToPDF`）也写这里——
    /// 二者复用 E4 的同一隔离目录（denylist 红线 / MOTW 由 `_download_loop` 在落盘事件上统一施加）。
    /// `None`（无沙箱：纯引擎冒烟）→ `save_as_pdf` 报 `Unsupported`（无落点）、`download` 仍触发但落
    /// chrome 默认行为（无沙箱时本就不该用）。
    download_dir: Option<String>,
    /// **SD-2 上传路径沙箱根**（per-pet 隔离 workspace 目录）。`act_upload_file` 在调
    /// `DOM.setFileInputFiles` 前逐路径 canonicalize + 包含判定：不在此目录下 ⇒
    /// `BrowserError::Blocked`（fail-closed）。`None`（无 per-pet 上下文，如纯引擎冒烟）⇒
    /// **一律拒绝上传**（fail-closed，default-deny）。
    workspace_dir: Option<PathBuf>,
    /// **出口防火墙 Host-owned runtime**（E5 + fail-closed 加固）——保活。防火墙循环对
    /// **每个** session（根 browser / page / OOPIF / **service_worker**）挂 `Fetch.enable` 全流量
    /// 拦截，订阅 `Fetch.requestPaused`，经 [`crate::firewall::decide`] 判定后 `continueRequest`
    /// 放行 / `failRequest` 阻断（IP 封禁硬阻）/（F1）升审批（跨域 POST-body）。**SW 必须也拦**
    /// （裁决⑪/不变量⑬）——P0 保持 SW attach，本循环对其 session 也 Fetch.enable。
    /// 固定 worker、两级有界队列、取消令牌与 watchdog 由本值统一所有；循环意外死亡时 fail
    /// 整条连接，backend Drop 时取消并 abort 整棵已登记任务树。
    _firewall_runtime: Option<FirewallLoopRuntime>,
    /// **P3-G1：注入的出口防火墙配置快照**（裁决①）。= `EngineConfig.firewall`（经
    /// from_launched / from_host 透传），**与防火墙循环持有的同一份配置**。仅供测试 accessor
    /// [`Self::firewall_config_for_test`] 读回断言「注入值真的到达引擎」（loop 在另一线程内消费，
    /// 无法直接观测）。**P3-D1 后 `FirewallConfig` 不再 `Copy`（改 `Clone`，因加了 `Vec` 域名策略字段）**，
    /// 存一份快照（`.clone()`，零热路径成本）。产品路径不读它（loop 才是真消费者）。
    firewall_config: crate::firewall::FirewallConfig,
    /// **P3-D2：per-session 已批准出口域集合**（决策3 always_allow）。与防火墙循环持有的
    /// 同一份（`Arc<Mutex<…>>` 共享）：审批一条被门控出口请求时若选「记住此域」（`EgressVerdict::
    /// ContinueAndRemember`），目标 eTLD+1 记进这里 → 同域后续出口请求不再悬挂审批直接放行。engine
    /// 生命周期内有效（非持久——持久域策略走 `FirewallConfig.allow_etld1` 的 secret 真值，X2）。
    /// backend 持有它仅为保活 + 测试 accessor；真消费者是 loop 的 spawn 审批任务。
    #[allow(dead_code)]
    approved_domains: crate::firewall::ApprovedDomains,
    /// **E3 evaluate 门控配置**（DESIGN §16「evaluate」/ 裁决⑨）：默认 [`crate::evaluate::EvaluateGate::default`]
    /// = **evaluate OFF**（`full_power=false`，default-deny）。act(Evaluate) 经 [`crate::evaluate::gate`]
    /// 据此判放行——**只看全权开关，绝不看 session_mode**（yolo/companion 无从豁免；不变量⑧）。
    /// **LIVE 读接线在 services.rs / F 阶段**（`read_bool_pref` 范式从 client_preferences 读全权开关灌进
    /// 来，使切换无需重启）；E3 引擎层先持默认 OFF 的门 + 纯逻辑放行判定。`persistent_login` 占位 false
    /// 待 P6（互斥逻辑已就位）。`AsyncMutex` 与其它共享态一致（F1 可在每次 act 前更新 LIVE 值）。
    evaluate_gate: AsyncMutex<crate::evaluate::EvaluateGate>,
    /// capabilities 快照。
    headful: bool,
    display_available: bool,
    /// **引擎级 observe⊥act 串行门**（DESIGN §22「observe 与 act 互斥」+「per-target act 串行」）。
    /// 跨 `navigate`/`screenshot`/`observe`/`act` 整个方法体持有：快照（observe）绝不与改 DOM 的动作
    /// 在同一引擎上交错（否则给模型陈旧 ref / 半应用页）。此前该不变量仅靠调用方串行成立
    /// （`is_concurrency_safe==false` → tool executor partition + 网关 `CompanionBrowser::lock`）；现在
    /// **引擎自身**保证——并发调用方也无法交错 observe/act。公平 `tokio::sync::Mutex`，只在单次已被
    /// 截止时间界定的操作内持有（每 CDP 命令超时 + `Progress` 截止 / `ACT_TIMEOUT`），绝不跨无界等待 → 不死锁。
    /// **作用域 per-Lane**（≠ per Chrome 进程）：共享 Host 的每个 lane adapter 都持不同 gate，因此
    /// 不同 Lane 可在同一 Connection 上并行；此锁绝不跨 Lane。重入安全：
    /// `navigate`/`screenshot`/`observe`/`act` 体内均只调 `*_impl`/`*_on_session`
    /// 助手、绝不回调这四个 trait 方法（已对抗式 grep 校验），故非重入锁不会自死锁。
    op_mutex: LaneOperationGate,
    /// Serializes lazy target replacement after the final tab crashes.
    /// Keeping recovery in its own short-lived gate guarantees one replacement
    /// target per Lane without introducing a Host-global lock.
    target_recovery_gate: AsyncMutex<()>,
    /// Known-secret exact-blackout registry (shared with facade via `Arc`). Debug serializers
    /// read this set and `String::replace` each value with `[KNOWN_SECRET_REDACTED]` before
    /// heuristic redaction passes. See [`crate::KnownSecretValues`] doc for invariants.
    known_secret_values: crate::KnownSecretValues,
}

impl Drop for CdpBackend {
    fn drop(&mut self) {
        if let Some(runtime) = self._firewall_runtime.as_ref() {
            runtime.abort();
        }
        // Dropping a Tokio JoinHandle detaches its task. Standalone backends
        // own these loops, and every loop retains a Connection clone (the tab
        // discovery loop also retains the tab registry), so they must be
        // explicitly aborted before cleanup authority is handed off.
        if let Some(loop_handle) = self._tab_discovery_loop.take() {
            loop_handle.abort();
        }
        if let Some(loop_handle) = self._download_loop.take() {
            loop_handle.abort();
        }
        if let Some(loop_handle) = self._attach_loop.take() {
            loop_handle.abort();
        }
        if let Some(cleanup) = self.lane_cleanup.as_ref()
            && !self.lane_closed.load(Ordering::Acquire)
        {
            cleanup.hand_off();
        }
        if let Some(process) = self._process.as_ref() {
            // The bounded target-cleanup worker retains the same process
            // authority. Hand it off explicitly so that a worker/job cycle
            // cannot postpone standalone Chromium teardown.
            process.hand_off();
        }
    }
}

fn validate_storage_state_for_restore(
    state: &crate::storage_state::StorageState,
) -> Result<(), BrowserError> {
    state.validate_bounds().map_err(|error| BrowserError::Blocked {
        reason: format!("storage_state restore exceeds its per-task hard boundary: {error}"),
    })
}

#[cfg(test)]
mod storage_state_restore_bound_tests {
    use super::*;

    #[test]
    fn oversized_state_is_rejected_before_any_browser_session_or_serialization() {
        let origin = crate::storage_state::OriginStorage::new_local_storage(
            "https://oversized.example",
            std::iter::empty::<(String, String)>(),
        );
        let state = crate::storage_state::StorageState {
            cookies: vec![],
            local_storage: vec![
                origin;
                crate::storage_state::MAX_STORAGE_STATE_ORIGINS + 1
            ],
        };
        assert!(matches!(
            validate_storage_state_for_restore(&state),
            Err(BrowserError::Blocked { .. })
        ));
    }
}

impl CdpBackend {
    /// 用一次成功的 [`crate::launch::launch_chrome`] 产物建后端：connect → 起 attach loop →
    /// enable_auto_attach → 取一个 page session。
    ///
    /// **编排铁律（Task B 约定）**：先 `run_attach_loop()`（装监听）再
    /// `enable_auto_attach()`（放行），否则首批子 session 的 attach 事件会丢。
    ///
    /// `#[allow(clippy::too_many_arguments)]`：本构造器逐参注入引擎配置（download/evaluate/firewall/
    /// egress 等都是 P3 各阶段一路打通的注入链真值，调用点仅集成测试 helper），
    /// 折成 config 结构会牵动 G1/D1/D2 多个已交付调用点的同步改动——超出本次重构范围。
    #[allow(clippy::too_many_arguments)]
    pub async fn from_launched(
        launched: Launched,
        headful: bool,
        display_available: bool,
        download_dir: Option<String>,
        workspace_dir: Option<PathBuf>,
        evaluate_full_power: bool,
        evaluate_persistent_login: bool,
        firewall: crate::firewall::FirewallConfig,
        egress_approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
        storage_state: Option<serde_json::Value>,
        known_secret_values: crate::KnownSecretValues,
        dns_resolver: Option<Arc<dyn crate::firewall::HostResolver>>,
    ) -> Result<Self, BrowserError> {
        // The historical direct constructor owned a separate lossy tab-discovery
        // broadcast loop. A popup burst could overrun that receiver and leave
        // untracked physical targets/renderers alive. Build the exact same
        // Host/router + Lane protocol used by production instead: one reliable
        // attach authority, one trusted task tab budget, and durable exact
        // target/process cleanup on every failure or cancellation seam.
        let download_staging_root = workspace_dir
            .as_deref()
            .map(crate::download::download_staging_root)
            .or_else(|| {
                download_dir
                    .as_deref()
                    .map(|dir| std::path::Path::new(dir).join(".staging"))
            });
        let host = CdpHostRuntime::from_launched(
            launched,
            headful,
            display_available,
            download_staging_root,
            download_dir,
            firewall,
            egress_approver,
            storage_state,
            dns_resolver,
        )
        .await?;

        let lane_id = "standalone".to_string();
        let resource_scope = crate::host::StandaloneResourceScope::new();
        let lane_authority = resource_scope.reserve_lane(lane_id.clone())?;
        let task_resource_key = resource_scope.task_resource_key().to_owned();
        let config = LaneEngineConfig {
            workspace_dir,
            evaluate_full_power,
            evaluate_persistent_login,
            known_secret_values: Some(known_secret_values),
            task_resource_key: Some(task_resource_key),
            max_task_tabs: crate::host::STANDALONE_MAX_LIVE_TABS_PER_SCOPE,
            task_tab_reservation_authority: Some(lane_authority.clone()),
            task_download_reservation_authority: Some(lane_authority),
        };

        Self::from_host(host, lane_id, config).await
    }
}

impl CdpBackend {
    /// Build one lane-scoped engine over an already connected shared host.
    pub(crate) async fn from_host(
        host: Arc<CdpHostRuntime>,
        lane_id: LaneId,
        config: LaneEngineConfig,
    ) -> Result<Self, BrowserError> {
        if host.shutdown.load(Ordering::Acquire) {
            return Err(BrowserError::SessionLost { recoverable: false });
        }
        host.router.cleanup_executor.ensure_accepting()?;
        let conn = host.conn.clone();
        let task_resource_key = config
            .task_resource_key
            .clone()
            .unwrap_or_else(|| lane_id.clone());
        let reliable_event_task_budget =
            ReliableEventTaskBudget::for_trusted_task(&task_resource_key);
        let task_tab_reservation_scope = config
            .task_tab_reservation_authority
            .clone()
            .map(|authority| TaskTabReservationScope {
                task_resource_key: task_resource_key.clone(),
                lane_id: lane_id.clone(),
                authority,
            });
        let task_download_reservation_scope = config
            .task_download_reservation_authority
            .clone()
            .map(|authority| TaskDownloadReservationScope {
                task_resource_key: task_resource_key.clone(),
                lane_id: lane_id.clone(),
                authority,
            });
        let initial_target_background =
            page_target_should_start_in_background(host.headful, host.display_available);
        // The CDP command and its nonce-correlated attach live in their own
        // task. Dropping this `from_host` future detaches that task; its output
        // still owns exact target cleanup and is closed when the task finishes.
        let pending_page = create_pending_page_session_owned(
            conn.clone(),
            Arc::clone(&host.router.cleanup_executor),
            Some(Arc::clone(&host.router)),
            initial_target_background,
            task_tab_reservation_scope.clone(),
        )
        .await?;
        let page_target_id = pending_page.target_id.clone();
        let page_session = pending_page.session_id.clone();
        let initial_tab = arm_tab(
            &conn,
            &page_target_id,
            &page_session,
            pending_page.task_tab_reservation.clone(),
        )
        .await?;
        let initial_frame_id = initial_tab.main_frame_id.clone();
        let initial_task_tab_reservation = initial_tab._task_tab_reservation.clone();
        let max_task_tabs = config.max_task_tabs;
        let download_dir = config
            .workspace_dir
            .as_deref()
            .map(crate::download::ensure_download_dir)
            .or_else(|| host.download_dir.clone());
        let mut tabs_map = HashMap::new();
        tabs_map.insert(page_target_id.clone(), initial_tab);
        let tabs = Arc::new(AsyncMutex::new(tabs_map));
        let active_target = Arc::new(AsyncMutex::new(page_target_id.clone()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        let lane_closing = Arc::new(AtomicBool::new(false));
        let lane_closed = Arc::new(AtomicBool::new(false));
        let lane_cleanup = LaneCleanupAuthority::new(
            conn.clone(),
            Arc::clone(&host.router.cleanup_executor),
            Arc::clone(&host.router),
            lane_id.clone(),
            Arc::clone(&lane_closing),
            Arc::clone(&tabs),
            page_target_id.clone(),
            page_session.clone(),
            initial_frame_id.clone(),
            initial_task_tab_reservation.as_ref(),
            task_tab_reservation_scope
                .as_ref()
                .map(|scope| Arc::clone(&scope.authority)),
        );
        let launch_guard = PendingLaneLaunchGuard::new(Arc::clone(&lane_cleanup));
        // From this synchronous transfer onward the Lane guard/backend is the
        // sole exact target cleanup authority.
        let _ = pending_page.transfer_to_lane();

        let registration_id = host
            .router
            .register_lane_with_resource_and_download_scope(
                lane_id.clone(),
                &tabs,
                &active_target,
                &active_frame,
                lane_closing.clone(),
                download_dir.clone(),
                Some(task_resource_key),
                max_task_tabs,
                config.task_tab_reservation_authority.clone(),
                config.task_download_reservation_authority.clone(),
            )
            .await
            .ok_or_else(|| {
                BrowserError::Other(format!(
                    "browser Lane {lane_id} is already registered or still cleaning up"
                ))
            })?;
        lane_cleanup.set_registration(registration_id);
        if !host.router.claim_target(&lane_id, &page_target_id).await {
            return Err(BrowserError::Other(format!(
                "new target could not be claimed for browser Lane {lane_id}"
            )));
        }
        host.router.claim_frame(&lane_id, &initial_frame_id).await;
        if let Err(error) = enable_fetch_on_session(&conn, &page_session).await {
            tracing::warn!(
                lane_id = %lane_id,
                session_id = %page_session,
                %error,
                "Fetch.enable on initial lane page failed"
            );
        }

        let known_secret_values = config.known_secret_values.unwrap_or_default();
        let backend = Self {
            conn,
            host: Some(host.clone()),
            #[cfg(test)]
            test_router: None,
            lane_id,
            task_tab_reservation_scope,
            task_download_reservation_scope,
            reliable_event_task_budget,
            lane_cleanup: Some(lane_cleanup),
            cleanup_executor: Arc::clone(&host.router.cleanup_executor),
            lane_closing,
            lane_closed,
            lane_retired: AtomicBool::new(false),
            lane_shutdown_gate: AsyncMutex::new(()),
            lane_close_confirmed: AsyncMutex::new(HashSet::new()),
            lane_cancel: CancellationToken::new(),
            tabs,
            active_target,
            active_frame,
            act_seq: std::sync::atomic::AtomicU64::new(0),
            _process: None,
            _attach_loop: None,
            _tab_discovery_loop: None,
            _download_loop: None,
            download_dir,
            workspace_dir: config.workspace_dir,
            _firewall_runtime: None,
            firewall_config: host.firewall_config.clone(),
            approved_domains: host.approved_domains.clone(),
            evaluate_gate: AsyncMutex::new(crate::evaluate::EvaluateGate {
                full_power: config.evaluate_full_power,
                persistent_login: config.evaluate_persistent_login,
            }),
            headful: host.headful,
            display_available: host.display_available,
            op_mutex: LaneOperationGate::default(),
            target_recovery_gate: AsyncMutex::new(()),
            known_secret_values,
        };

        if let Some(value) = host.storage_state.clone() {
            match crate::storage_state::StorageState::from_json(value) {
                Ok(state) => {
                    if let Err(error) = backend.restore_cookies(&state).await {
                        tracing::warn!(%error, "shared-host cookie restore failed");
                    }
                    if let Err(error) = backend.restore_local_storage(&state).await {
                        tracing::warn!(%error, "shared-host localStorage restore failed");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "shared-host storage state is invalid");
                }
            }
        }
        // The returned backend retains the same final-Drop authority. This
        // commit therefore does not open a gap before coordinator publication.
        launch_guard.commit();
        Ok(backend)
    }

    fn target_router(&self) -> Option<&Arc<HostTargetRouter>> {
        if let Some(host) = self.host.as_ref() {
            return Some(&host.router);
        }
        #[cfg(test)]
        {
            return self.test_router.as_ref();
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    pub(crate) async fn lock_operations_for_task_policy(
        &self,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        self.op_mutex.lock_owned().await
    }

    /// Close one exact top-level target selected by the router's committed
    /// task policy. Missing records mean an asynchronous target-loss event
    /// already converged that exact page. All other failures retain exact
    /// ownership and are handed to the bounded target cleanup executor by
    /// `close_tab_impl`.
    pub(crate) async fn close_tab_for_task_policy(
        &self,
        target_id: &str,
    ) -> Result<(), BrowserError> {
        if self.lane_closing.load(Ordering::Acquire) {
            return Err(BrowserError::TargetClosed);
        }
        if !self.tabs.lock().await.contains_key(target_id) {
            if let Some(router) = self.target_router()
                && router.has_task_tab_reservation(target_id).await
            {
                router
                    .schedule_owned_target_cleanup(&self.lane_id, target_id)
                    .await;
                return Err(BrowserError::Other(
                    "browser target is absent from its Lane registry but remains cleanup-pending"
                        .into(),
                ));
            }
            return Ok(());
        }
        let progress = Progress::new(TARGET_CLEANUP_JOB_BUDGET);
        self.close_tab_impl(target_id, &progress).await.map(|_| ())
    }

    /// Transfer a failed coordinator close to the Host's bounded cleanup
    /// executor. The authority is single-use, so a later backend Drop cannot
    /// enqueue the same Lane twice. A managed Lane should always carry this
    /// authority; violating that invariant poisons the Host instead of
    /// silently abandoning its targets.
    pub(crate) fn hand_off_lane_cleanup(&self) {
        if let Some(cleanup) = self.lane_cleanup.as_ref() {
            cleanup.hand_off();
        } else {
            tracing::error!(
                lane_id = %self.lane_id,
                "managed Lane lost its cleanup authority; escalating Host cleanup"
            );
            self.cleanup_executor.poison(None, false);
        }
    }

    /// Cancel in-flight work and close only this lane's targets.  This method
    /// deliberately bypasses `op_mutex`, so a hung lane operation cannot block
    /// authoritative cleanup.
    pub(crate) async fn shutdown_lane(&self) -> Result<(), BrowserError> {
        let _shutdown = self.lane_shutdown_gate.lock().await;
        if self.lane_closed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.lane_closing.store(true, Ordering::Release);
        self.lane_cancel.cancel();
        if !self.lane_retired.load(Ordering::Acquire) {
            let mut drain_passes = 0usize;
            loop {
                let mut target_ids = {
                    let tabs = self.tabs.lock().await;
                    tabs.keys().cloned().collect::<Vec<_>>()
                };
                if let Some(router) = self.target_router() {
                    target_ids.extend(router.owned_targets(&self.lane_id).await);
                }
                target_ids.sort();
                target_ids.dedup();
                let pending_targets = {
                    let confirmed = self.lane_close_confirmed.lock().await;
                    target_ids
                        .into_iter()
                        .filter(|target_id| !confirmed.contains(target_id))
                        .collect::<Vec<_>>()
                };
                for target_id in pending_targets {
                    if let Err(error) =
                        close_target_or_confirm_absent(&self.conn, &target_id).await
                    {
                        tracing::debug!(
                            lane_id = %self.lane_id,
                            target_id_suffix = %cdp_id_suffix(&target_id),
                            %error,
                            "closing lane target failed; preserving lane state for retry"
                        );
                        return Err(error);
                    }
                    self.lane_close_confirmed
                        .lock()
                        .await
                        .insert(target_id.clone());
                    if let Some(record) = self.tabs.lock().await.get(&target_id) {
                        abort_tab_record(record);
                    }
                }

                let Some(router) = self.target_router() else {
                    break;
                };
                let live_targets = router
                    .claim_live_targets_for_closing_lane(&self.lane_id)
                    .await?;
                if live_targets.is_empty() {
                    break;
                }
                drain_passes += 1;
                if drain_passes >= LATE_TARGET_DRAIN_MAX_PASSES {
                    return Err(BrowserError::Other(
                        "closing Lane still has live top-level targets after bounded drain".into(),
                    ));
                }
                let mut confirmed = self.lane_close_confirmed.lock().await;
                for target_id in live_targets {
                    confirmed.remove(&target_id);
                }
                drop(confirmed);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            // Retirement is a durable phase transition. `unregister_lane` is
            // idempotent, so cancellation at any later await can safely retry
            // it without re-entering the active-Lane inventory drain.
            self.lane_retired.store(true, Ordering::Release);
        }
        if let Some(router) = self.target_router() {
            if let Some(registration_id) = self
                .lane_cleanup
                .as_ref()
                .and_then(|cleanup| cleanup.registration())
            {
                let _ = router
                    .unregister_lane_if_current(&self.lane_id, registration_id)
                    .await;
            } else {
                #[cfg(test)]
                router.unregister_lane(&self.lane_id).await;
            }
            router.finalize_retired_lane(&self.lane_id).await?;
        }
        let records = {
            let mut tabs = self.tabs.lock().await;
            std::mem::take(&mut *tabs)
        };
        for record in records.values() {
            abort_tab_record(record);
        }
        self.lane_close_confirmed.lock().await.clear();
        self.lane_closed.store(true, Ordering::Release);
        if let Some(cleanup) = self.lane_cleanup.as_ref() {
            cleanup.mark_finished();
        }
        Ok(())
    }

    /// **active tab 句柄快照**（D1 锁模式核心；DESIGN §13 裁决⑥ + [`crate::tabs`] 锁设计）：短暂锁
    /// `active_target` + `tabs`，从 active [`TabRecord`] **克隆出**所有可独立持有的句柄
    /// （[`TabHandles`]），**立即释放两把锁**后返回。observe/act/navigate 全程用克隆出的句柄操作——
    /// **绝不**跨 await 持 `tabs` 锁（否则阻塞 D3 tab 发现循环 + observe 内嵌套锁 ref_table 死锁）。
    ///
    /// `active_target` 指向的 tab 已崩溃/不在 `tabs` 时，先剔除崩溃记录并按 target id
    /// 确定性选择仍存活的 tab；没有 survivor 时按需创建 replacement target。整条恢复链由
    /// lane-local gate 串行化，因此并发访问只会发布一个 replacement。
    pub(crate) async fn active_tab_handles(&self) -> Result<TabHandles, BrowserError> {
        if self.lane_closing.load(Ordering::Acquire) {
            return Err(BrowserError::TargetClosed);
        }

        // Agent operations are already serialized by `op_mutex`; this
        // additional Lane-local gate makes final-tab replacement single-flight.
        let _recovery = self.target_recovery_gate.lock().await;
        if self.lane_closing.load(Ordering::Acquire) {
            return Err(BrowserError::TargetClosed);
        }
        if let Some(handles) = self.select_live_active_tab().await {
            return Ok(handles);
        }
        self.create_replacement_target().await
    }

    /// Resolve the current active target, pruning sticky-crashed records and
    /// deterministically selecting another Lane-owned tab when available.
    async fn select_live_active_tab(&self) -> Option<TabHandles> {
        let previous_active = self.active_target.lock().await.clone();
        let (selected, removed) = {
            let mut tabs = self.tabs.lock().await;
            let crashed = tabs
                .iter()
                .filter_map(|(target_id, record)| {
                    self.conn
                        .registry()
                        .is_session_crashed(&record.session_id)
                        .then(|| target_id.clone())
                })
                .collect::<Vec<_>>();
            let mut removed = Vec::with_capacity(crashed.len());
            for target_id in crashed {
                if let Some(record) = tabs.remove(&target_id) {
                    removed.push(record);
                }
            }

            let selected_id = tabs
                .contains_key(&previous_active)
                .then(|| previous_active.clone())
                .or_else(|| deterministic_survivor(tabs.keys().map(String::as_str), ""));
            let selected = selected_id
                .as_ref()
                .and_then(|target_id| tabs.get(target_id))
                .map(tab_handles);
            (selected, removed)
        };
        for record in removed {
            abort_tab_record(&record);
        }

        if let Some(handles) = selected {
            if handles.target_id != previous_active {
                *self.active_target.lock().await = handles.target_id.clone();
                *self.active_frame.lock().await = None;
            }
            Some(handles)
        } else {
            None
        }
    }

    /// Lazily create the replacement required after a Lane loses its final
    /// target. The recovery gate held by [`Self::active_tab_handles`] ensures
    /// concurrent callers create at most one target.
    async fn create_replacement_target(&self) -> Result<TabHandles, BrowserError> {
        if self.conn.registry().is_connection_closed() {
            return Err(BrowserError::SessionLost { recoverable: false });
        }

        let replacement_target_background =
            page_target_should_start_in_background(self.headful, self.display_available);
        let pending_page = create_pending_page_session_owned(
            self.conn.clone(),
            Arc::clone(&self.cleanup_executor),
            self.target_router().cloned(),
            replacement_target_background,
            self.task_tab_reservation_scope.clone(),
        )
        .await?;
        let target_id = pending_page.target_id.clone();
        let session_id = pending_page.session_id.clone();
        if let Some(host) = &self.host
            && !host.router.claim_target(&self.lane_id, &target_id).await
        {
            return Err(BrowserError::TargetCrashed);
        }

        // The router or standalone discovery loop may have armed the target
        // while `claim_target` awaited a quarantined attach.
        if let Some(handles) = self
            .tabs
            .lock()
            .await
            .get(&target_id)
            .map(tab_handles)
        {
            let _ = pending_page.transfer_to_lane();
            *self.active_target.lock().await = target_id;
            *self.active_frame.lock().await = None;
            return Ok(handles);
        }

        let record = arm_tab(
            &self.conn,
            &target_id,
            &session_id,
            pending_page.task_tab_reservation.clone(),
        )
        .await?;
        let handles = tab_handles(&record);

        if self.conn.registry().is_session_crashed(&session_id) {
            abort_tab_record(&record);
            return Err(BrowserError::TargetCrashed);
        }
        if let Some(host) = &self.host
            && host.router.is_target_lost(&target_id).await
        {
            abort_tab_record(&record);
            return Err(BrowserError::TargetCrashed);
        }

        let survivor = deterministic_survivor(
            self.tabs.lock().await.keys().map(String::as_str),
            &target_id,
        );

        if let Some(survivor) = survivor {
            abort_tab_record(&record);
            close_target_or_confirm_absent(&self.conn, &target_id).await?;
            if let Some(router) = self.target_router() {
                router.release_target(&target_id, None).await;
            }
            let _ = pending_page.transfer_to_lane();
            *self.active_target.lock().await = survivor;
            *self.active_frame.lock().await = None;
            return self
                .select_live_active_tab()
                .await
                .ok_or(BrowserError::TargetClosed);
        }

        if let Some(host) = &self.host {
            match host
                .router
                .publish_armed_page(
                    &self.lane_id,
                    PendingPage {
                        target_id: target_id.clone(),
                        session_id: session_id.clone(),
                        opener_target_id: None,
                        target_url: None,
                    },
                    record,
                )
                .await
            {
                OwnedPagePublish::Inserted => {}
                OwnedPagePublish::AlreadyPresent => {
                    let _ = pending_page.transfer_to_lane();
                    return self
                        .select_live_active_tab()
                        .await
                        .ok_or(BrowserError::TargetClosed);
                }
                OwnedPagePublish::RejectedCapacity => {
                    let _ = pending_page.transfer_to_lane();
                    return Err(BrowserError::Blocked {
                        reason: "this task reached its browser tab limit; close another task tab before recovering this Lane".into(),
                    });
                }
                OwnedPagePublish::RejectedState => return Err(BrowserError::TargetClosed),
            }
        } else {
            self.tabs.lock().await.insert(target_id.clone(), record);
        }

        let _ = pending_page.transfer_to_lane();
        *self.active_target.lock().await = target_id.clone();
        *self.active_frame.lock().await = None;
        if let Err(error) = enable_fetch_on_session(&self.conn, &session_id).await {
            tracing::warn!(
                lane_id = %self.lane_id,
                target_id_suffix = %cdp_id_suffix(&target_id),
                %error,
                "Fetch.enable on replacement Lane target failed"
            );
        }
        tracing::info!(
            target: "nomi_browser_engine::host",
            lane_id = %self.lane_id,
            target_id_suffix = %cdp_id_suffix(&target_id),
            "created replacement target after the Lane lost its final tab"
        );
        Ok(handles)
    }

    /// **Takeover seam: bring the headful browser window to the foreground.**
    ///
    /// - Headful + display available → resolves the active target's real browser window,
    ///   restores it to `normal`, activates the active target/tab, then delivers
    ///   renderer/document focus via `Page.bringToFront`. Returns `Ok(())`.
    /// - Headless or no display → returns `Err(BrowserError::Unsupported)` with
    ///   capability="takeover" so the caller can map it to [`TakeoverResolution::Unavailable`].
    ///
    /// Does NOT hold `op_mutex` — this is a pure window-management command that does
    /// not interact with observe/act serialization (mirrors `activateTarget` in switch_tab).
    pub async fn bring_to_front(&self) -> Result<(), BrowserError> {
        if !self.headful || !self.display_available {
            return Err(BrowserError::Unsupported {
                capability: "takeover".into(),
                hint: "headful window required but engine is headless or no display available"
                    .into(),
            });
        }

        // Resolve both handles before issuing any window-management command. The
        // target id is required because this command runs on the browser session.
        let handles = self.active_tab_handles().await?;
        use chromiumoxide::cdp::browser_protocol::browser::{
            Bounds, GetWindowForTargetParams, GetWindowForTargetReturns, SetWindowBoundsParams,
            WindowState,
        };
        use chromiumoxide::cdp::browser_protocol::target::ActivateTargetParams;

        // Ask Chromium for the native window which owns this target and
        // normalize it before activation. Headful Hosts are created only by an
        // explicit trusted presentation transition; this is not a mechanism
        // for simulating headless work with a minimized window.
        let window = self
            .conn
            .send::<GetWindowForTargetParams>(
                ROOT_SESSION,
                &GetWindowForTargetParams::builder()
                    .target_id(handles.target_id.clone())
                    .build(),
            )
            .await
            .map_err(map_transport_err)?;
        let window: GetWindowForTargetReturns = serde_json::from_value(window)
            .map_err(|_| BrowserError::Other("invalid Browser.getWindowForTarget response".into()))?;
        let restore = SetWindowBoundsParams::new(
            window.window_id,
            Bounds::builder().window_state(WindowState::Normal).build(),
        );
        let _ = self
            .conn
            .send::<SetWindowBoundsParams>(ROOT_SESSION, &restore)
            .await
            .map_err(map_transport_err)?;

        // Activate only after the native window has been restored so Chromium can
        // select the requested tab in a visible window. Unlike switch_tab's
        // cosmetic best-effort activation, this trusted foreground seam reports a
        // failure if activation itself fails.
        let _ = self
            .conn
            .send::<ActivateTargetParams>(
                ROOT_SESSION,
                &ActivateTargetParams::new(handles.target_id.clone()),
            )
            .await
            .map_err(map_transport_err)?;

        // F37: Page.bringToFront is the one command which delivers renderer/
        // document focus (WebContents Activate()+Focus()); Target.activateTarget
        // alone only selects the tab, so document.hasFocus() stays false and
        // focus-gated widgets (OTP/clipboard/login) never react. Issue it last,
        // on the page session, once the window is restored and the tab selected
        // — this was the old implementation's primary foreground command.
        use chromiumoxide::cdp::browser_protocol::page::BringToFrontParams;
        let _ = self
            .conn
            .send::<BringToFrontParams>(&handles.session_id, &BringToFrontParams::default())
            .await
            .map_err(map_transport_err)?;

        Ok(())
    }
}

/// **arm 一个 tab（D3 复用核心）**：为给定 `(target_id, session_id)` 物化注入管线（utility world +
/// 现存帧补建 + context 登记循环）、读权威主帧 id、接 OOPIF arm 循环，建好一个完整的 [`TabRecord`]。
///
/// 初始 tab 与 Host 路由器发现的新顶层 page **共用本 helper**——同一套 arm 逻辑，零分叉。
/// 返回的 [`TabRecord`] 持有 `_inject_loop`/`_oopif_loop`
/// 两个后台 `JoinHandle`：它们订阅**全局共享连接**的 broadcast，靠 `RecvError::Closed` 退出——但**连接
/// 关单个 tab 时仍存活**，故关 tab 仅从 `tabs` 移除 TabRecord（drop 这俩 handle）**不会**让循环退出
/// （drop 是 detach 非 abort）。**close_tab 必须显式 `.abort()` 这俩 handle**（见 [`CdpBackend::close_tab_impl`]）。
///
/// 错误：injection arm / 读主帧失败 → 映射为 [`BrowserError`]（绝不 panic）。
async fn arm_tab(
    conn: &Connection,
    target_id: &str,
    session_id: &str,
    task_tab_reservation: Option<Arc<dyn TaskTabReservation>>,
) -> Result<TabRecord, BrowserError> {
    // page session 注入管线：new + arm（物化 utility world + 现存帧补建 + 起 context 登记循环）。
    // 保活其 loop 句柄，否则 world 创建事件不再被收下。
    let injection = InjectionManager::new(conn.clone(), session_id.to_string());
    let inject_loop = AbortOnDropTask::new(injection.arm().await.map_err(map_inject_err)?);

    // 主 frameId = page target 的 targetId（CDP 约定）。从 page session 的 frameTree 读权威主帧 id
    // （与 targetId 一致，但不依赖外部传入）。
    let main_frame_id = injection.main_frame_id().await.map_err(map_inject_err)?;

    // OOPIF 子 session arm 接线骨架：后台订阅 attachedToTarget，对 iframe 类型的子 session（非本 page
    // session）arm 一个 InjectionManager 入 oopif_managers。真跨源 OOPIF 须 http fixture 后续验
    // （见 `TODO(verify-oopif)`）。每 tab 自有一份 oopif_managers + 一条 arm 循环（per-tab 隔离）。
    let oopif_managers: std::sync::Arc<Mutex<HashMap<String, OopifEntry>>> =
        std::sync::Arc::new(Mutex::new(HashMap::new()));
    let oopif_loop =
        spawn_oopif_arm_loop(conn.clone(), session_id.to_string(), oopif_managers.clone());

    // 调试捕获：enable Runtime/Log + 起长驻 drain 循环写入有界缓冲。
    // Network.enable 在 navigate 内已幂等调用，此处也 enable 确保非导航期事件亦被捕获。
    let debug_buffers = std::sync::Arc::new(std::sync::Mutex::new(
        crate::debug_capture::DebugBuffers::default(),
    ));
    let debug_loop = spawn_debug_capture_loop(
        conn.clone(),
        session_id.to_string(),
        debug_buffers.clone(),
    );

    Ok(TabRecord {
        _task_tab_reservation: task_tab_reservation,
        target_id: target_id.to_string(),
        session_id: session_id.to_string(),
        injection,
        _inject_loop: inject_loop.into_inner(),
        main_frame_id,
        oopif_managers,
        _oopif_loop: oopif_loop,
        ref_table: std::sync::Arc::new(AsyncMutex::new(None)),
        debug: debug_buffers,
        _debug_loop: debug_loop,
    })
}

/// **tab 发现后台循环（D3，DESIGN §13 + 裁决⑥/不变量⑮）**：订阅 `Target.attachedToTarget`（全 session
/// 通配），对**新顶层 page**（`type=="page"`，非主 page session，不在 `tabs`）调 [`arm_tab`] 建
/// [`TabRecord`] 入 `tabs`——**不抢焦点、不改 active**（返「新标签已打开[last4]」让 LLM 显式 switch，
/// browser-use 策略 / DESIGN:188）。
///
/// **与 OOPIF arm 循环（[`spawn_oopif_arm_loop`]）的协调防重复 arm**：本循环 [`crate::tabs::should_arm_as_page`]
/// **只收 `type=="page"`**；OOPIF 循环只收 `type=="iframe"`。二者各自筛 type，互不重叠——同一 attach 事件
/// 绝不被两路同时 arm。再加「不在 tabs」守卫（CDP 对同 target 多次 attach 时不重复 arm，`tabs` map 无重复 key）。
///
/// **不等子 session 放行**：`run_attach_loop`（全局 attach loop）已先登记子 session 并放行
/// （runIfWaitingForDebugger）。本循环 arm 前轮询确认子 session 已登记（仿 OOPIF 循环），再物化注入管线。
///
/// 所有错误 best-effort：单个 tab arm 失败只 warn 不影响其它，**绝不 panic**。连接关闭（`RecvError::Closed`）
/// → 退出循环（backend Drop 关连接即触发）。
fn spawn_tab_discovery_loop(
    conn: Connection,
    main_page_session: String,
    tabs: Arc<AsyncMutex<HashMap<String, TabRecord>>>,
) -> tokio::task::JoinHandle<()> {
    use chromiumoxide::cdp::browser_protocol::target::EventAttachedToTarget;
    let mut rx = conn.subscribe(EventAttachedToTarget::IDENTIFIER, None);
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let Ok(att) =
                        serde_json::from_value::<EventAttachedToTarget>(ev.params.clone())
                    else {
                        continue;
                    };
                    let sid: String = att.session_id.clone().into();
                    let ttype = att.target_info.r#type.clone();
                    let tid: String = att.target_info.target_id.clone().into();

                    // type 分流（防与 OOPIF 循环重复 arm）+ 非主 session + 不在 tabs（短临界区查后释放锁）。
                    let already = tabs.lock().await.contains_key(&tid);
                    let is_main = sid == main_page_session;
                    if !crate::tabs::should_arm_as_page(&ttype, is_main, already) {
                        continue;
                    }

                    // 等子 session 在注册表登记（run_attach_loop 的 handle_attached 负责登记 + 放行）。
                    let deadline = tokio::time::Instant::now() + OOPIF_SESSION_REGISTER_TIMEOUT;
                    let registry = conn.registry().clone();
                    while !registry.has_session(&sid) {
                        if tokio::time::Instant::now() >= deadline {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    if !registry.has_session(&sid) {
                        tracing::warn!(
                            target: "nomi_browser_engine::backend::cdp",
                            session_id = %sid,
                            target_id_suffix = %cdp_id_suffix(&tid),
                            "new page child session never registered; skip arm"
                        );
                        continue;
                    }

                    // arm 成 TabRecord（复用 arm_tab）。失败 best-effort：warn 后继续（不影响已有 tab）。
                    match arm_tab(&conn, &tid, &sid, None).await {
                        Ok(record) => {
                            // 再次确认未被并发插入（双查，避免两条 attach 事件窗口竞态重 arm）。
                            let mut guard = tabs.lock().await;
                            if guard.contains_key(&tid) {
                                // 已被插入：丢弃本次 record，经共享 helper 全量 abort 其
                                // **三个**后台循环（F52：此前漏 abort `_debug_loop`，泄漏一条
                                // 长驻订阅 Runtime/Log/Network 事件的调试捕获任务）。
                                abort_tab_record(&record);
                                continue;
                            }
                            let target_id_suffix = cdp_id_suffix(&tid);
                            guard.insert(tid.clone(), record);
                            drop(guard);
                            // **不抢焦点、不改 active**：只记日志（LLM 经 tabs/switch_tab 显式切换）。
                            tracing::info!(
                                target: "nomi_browser_engine::backend::cdp",
                                target_id_suffix = %target_id_suffix,
                                "新标签已打开[{target_id_suffix}]（未抢焦点；observe/act 仍在原标签，需显式 switch_tab）"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "nomi_browser_engine::backend::cdp",
                                target_id_suffix = %cdp_id_suffix(&tid), error = %e,
                                "arm discovered tab failed (non-fatal)"
                            );
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}


/// 2. **再** `Target.createTarget{url:"about:blank"}` 拿 targetId；
/// 3. 等订阅里出现 `target_info.target_id == targetId` 的 attach 事件 → 取其 sessionId。
///
/// flatten auto-attach（enable_auto_attach 已开）会自动 attach 新 page，故无需手动 attach。
/// attach loop 会**同时**登记该子 session 并放行；本函数只需拿到 (targetId, sessionId)。
///
/// **D1**：返回 `(target_id, session_id)`——targetId 是 tabs 注册表的 key + active_target 指针
/// （`createTarget` 回包已给 targetId，attach 事件的 `target_info.target_id` 与之一致；二者择一即可，
/// 这里直接复用 createTarget 拿到的 `target_id`）。
const fn page_target_should_start_in_background(
    headful: bool,
    display_available: bool,
) -> bool {
    !(headful && display_available)
}

fn initial_page_target_params(background: bool) -> CreateTargetParams {
    let mut params = CreateTargetParams::new("about:blank");
    params.background = Some(background);
    params
}

static NEXT_PENDING_PAGE_NONCE: AtomicU64 = AtomicU64::new(1);

fn pending_page_url() -> String {
    let nonce = NEXT_PENDING_PAGE_NONCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "about:blank#nomifun-pending-{}-{nonce}",
        std::process::id()
    )
}

fn pending_page_target_params(url: &str, background: bool) -> CreateTargetParams {
    let mut params = CreateTargetParams::new(url);
    params.background = Some(background);
    params
}

fn pending_page_from_attach(
    event: &crate::transport::CdpEvent,
    pending_url: &str,
    response_target_id: Option<&str>,
) -> Option<(String, String)> {
    let attached = serde_json::from_value::<EventAttachedToTarget>(event.params.clone()).ok()?;
    if attached.target_info.r#type != "page" {
        return None;
    }
    let target_id: String = attached.target_info.target_id.into();
    let url_matches = attached.target_info.url == pending_url;
    if !url_matches && response_target_id != Some(target_id.as_str()) {
        return None;
    }
    Some((target_id, attached.session_id.into()))
}

async fn hand_off_pending_page_error(
    conn: &Connection,
    executor: &Arc<TargetCleanupExecutor>,
    router: &Option<Arc<HostTargetRouter>>,
    identity: Option<(String, Option<String>)>,
    pending_url: &str,
    task_tab_reservation: Option<Arc<dyn TaskTabReservation>>,
    lane_resource_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    error: BrowserError,
) -> BrowserError {
    // An absent inventory after a timed-out create command is not an ordering
    // proof: Chromium may execute the command later. Preserve the unknown
    // nonce as a cleanup intent; the bounded executor escalates it directly to
    // whole-Host teardown rather than accepting two short empty snapshots.
    if let Some((target_id, session_id)) = identity {
        PendingCreatedPageCleanup::new(
            conn.clone(),
            Arc::clone(executor),
            router.clone(),
            Some(target_id),
            session_id,
            Some(pending_url.to_string()),
            task_tab_reservation,
            lane_resource_authority,
        )
        .hand_off();
    } else {
        PendingCreatedPageCleanup::new(
            conn.clone(),
            Arc::clone(executor),
            router.clone(),
            None,
            None,
            Some(pending_url.to_string()),
            task_tab_reservation,
            lane_resource_authority,
        )
        .hand_off();
    }
    error
}

/// Cancellation-safe shared-Lane page creation.
///
/// The caller executes this future in an owned task and supplies a token which
/// is cancelled when the waiting `from_host` future is dropped. A unique inert
/// URL correlates `attachedToTarget` even when Chromium executes
/// `Target.createTarget` but its response is withheld or lost.
async fn create_pending_lane_page_session(
    conn: Connection,
    executor: Arc<TargetCleanupExecutor>,
    router: Option<Arc<HostTargetRouter>>,
    background: bool,
    caller_cancelled: CancellationToken,
    task_tab_reservation_scope: Option<TaskTabReservationScope>,
) -> Result<PendingCreatedPage, BrowserError> {
    let pending_url = pending_page_url();
    // The cross-Host slot is acquired before Target.createTarget. Its stable
    // nonce key makes retransmitted/duplicate attach handling idempotent.
    let lane_resource_authority = task_tab_reservation_scope
        .as_ref()
        .map(|scope| Arc::clone(&scope.authority));
    let task_tab_reservation = match task_tab_reservation_scope.as_ref() {
        Some(scope) => Some(
            scope
                .authority
                .reserve(&scope.task_resource_key, &scope.lane_id, &pending_url)
                .await?,
        ),
        None => None,
    };
    let mut attached_rx = conn.subscribe(EventAttachedToTarget::IDENTIFIER, None);
    let deadline = tokio::time::Instant::now() + PENDING_PAGE_CREATE_RECOVERY_TIMEOUT;
    if let Some(router) = router.as_ref() {
        router
            .register_pending_create(
                &pending_url,
                deadline,
                task_tab_reservation.clone(),
                task_tab_reservation_scope.as_ref(),
            )
            .await?;
    }
    let params = pending_page_target_params(&pending_url, background);
    let create = conn.send::<CreateTargetParams>(ROOT_SESSION, &params);
    tokio::pin!(create);

    let mut response_target_id: Option<String> = None;
    let mut response_error: Option<BrowserError> = None;
    let mut attached_identity: Option<(String, String)> = None;
    let mut caller_is_cancelled = false;
    let mut attached_closed = false;

    loop {
        if let Some((target_id, session_id)) = attached_identity.as_ref()
            && (response_target_id.as_deref() == Some(target_id.as_str())
                || response_error.is_some()
                || caller_is_cancelled)
        {
            if caller_is_cancelled || response_error.is_some() {
                let error = response_error.take().unwrap_or(BrowserError::Other(
                    "shared Host Lane page launch was cancelled".into(),
                ));
                return Err(
                    hand_off_pending_page_error(
                        &conn,
                        &executor,
                        &router,
                        Some((target_id.clone(), Some(session_id.clone()))),
                        &pending_url,
                        task_tab_reservation.clone(),
                        lane_resource_authority.clone(),
                        error,
                    )
                    .await,
                );
            }
            let cleanup = PendingCreatedPageCleanup::new(
                conn.clone(),
                Arc::clone(&executor),
                router.clone(),
                Some(target_id.clone()),
                Some(session_id.clone()),
                Some(pending_url.clone()),
                task_tab_reservation.clone(),
                lane_resource_authority.clone(),
            );
            return Ok(PendingCreatedPage {
                target_id: target_id.clone(),
                session_id: session_id.clone(),
                cleanup,
                task_tab_reservation: task_tab_reservation.clone(),
                transferred: false,
            });
        }
        if attached_identity.is_none()
            && (response_error.is_some()
                || (caller_is_cancelled && response_target_id.is_some()))
        {
            let error = response_error.take().unwrap_or(BrowserError::Other(
                "shared Host Lane page launch was cancelled".into(),
            ));
            let identity = response_target_id
                .take()
                .map(|target_id| (target_id, None));
            return Err(
                hand_off_pending_page_error(
                    &conn,
                    &executor,
                    &router,
                    identity,
                    &pending_url,
                    task_tab_reservation.clone(),
                    lane_resource_authority.clone(),
                    error,
                )
                .await,
            );
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            let identity = attached_identity
                .take()
                .map(|(target_id, session_id)| (target_id, Some(session_id)))
                .or_else(|| {
                    response_target_id
                        .take()
                        .map(|target_id| (target_id, None))
                });
            let error = response_error.take().unwrap_or_else(|| {
                BrowserError::Other(format!(
                    "timed out creating or attaching pending Lane page {pending_url}"
                ))
            });
            return Err(
                hand_off_pending_page_error(
                    &conn,
                    &executor,
                    &router,
                    identity,
                    &pending_url,
                    task_tab_reservation.clone(),
                    lane_resource_authority.clone(),
                    error,
                )
                .await,
            );
        }

        tokio::select! {
            response = &mut create, if response_target_id.is_none() && response_error.is_none() => {
                match response {
                    Ok(result) => {
                        response_target_id = result
                            .get("targetId")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned);
                        if response_target_id.is_none() {
                            response_error = Some(BrowserError::Other(
                                "createTarget response missing targetId".into(),
                            ));
                        }
                    }
                    Err(error) => response_error = Some(map_transport_err(error)),
                }
            }
            event = attached_rx.recv(), if !attached_closed => {
                match event {
                    Ok(event) => {
                        if let Some(identity) = pending_page_from_attach(
                            &event,
                            &pending_url,
                            response_target_id.as_deref(),
                        ) {
                            attached_identity = Some(identity);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        attached_closed = true;
                        response_error.get_or_insert(BrowserError::SessionLost {
                            recoverable: false,
                        });
                    }
                }
            }
            _ = caller_cancelled.cancelled(), if !caller_is_cancelled => {
                caller_is_cancelled = true;
            }
            _ = tokio::time::sleep(remaining) => {}
        }
    }
}

async fn create_page_session(
    conn: &Connection,
    background: bool,
) -> Result<(String, String), BrowserError> {
    // 1) 先订阅 attach 事件（在 createTarget 之前，避免错过）。
    let mut attached_rx = conn.subscribe(EventAttachedToTarget::IDENTIFIER, None);

    // 2) 在根 session 上建 page target（默认 browser context）。普通 Agent Lane
    // 运行在真正的 Headless Host，仍显式要求后台 target。受信任的 external
    // 展示入口会启动有效的 Headful Host；它的首个或最终标签恢复 target 必须前台
    // 创建，否则 `--no-startup-window` 后 Chromium 可能只有一个后台 target，用户
    // 看不到 Browser Use 实际工作的窗口。
    let params = initial_page_target_params(background);
    let result = conn
        .send::<CreateTargetParams>(ROOT_SESSION, &params)
        .await
        .map_err(map_transport_err)?;
    let target_id: String = result
        .get("targetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BrowserError::Other("createTarget response missing targetId".into()))?
        .to_string();

    // 3) 等到该 targetId 的 attach 事件，取 sessionId。
    let deadline = tokio::time::Instant::now() + PAGE_ATTACH_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(BrowserError::Other(format!(
                "timed out waiting for attachedToTarget of page target {target_id}"
            )));
        }
        match tokio::time::timeout(remaining, attached_rx.recv()).await {
            Ok(Ok(ev)) => {
                // 只认 page 类型且 targetId 匹配的 attach。
                let parsed: Result<EventAttachedToTarget, _> =
                    serde_json::from_value(ev.params.clone());
                if let Ok(att) = parsed {
                    let tid: String = att.target_info.target_id.clone().into();
                    if tid == target_id && att.target_info.r#type == "page" {
                        return Ok((target_id, att.session_id.into()));
                    }
                }
                // 非目标事件（其它 target 的 attach）：继续等。
            }
            // 广播落后（lagged）→ 继续收（可能错过，但下个匹配仍能拿到；超时兜底）。
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            // 连接关闭。
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Err(BrowserError::SessionLost { recoverable: false });
            }
            // 超时。
            Err(_elapsed) => {
                return Err(BrowserError::Other(format!(
                    "timed out waiting for attachedToTarget of page target {target_id}"
                )));
            }
        }
    }
}

/// OOPIF 子 session 等子 session 已登记的轮询上限（`run_attach_loop` 先登记再放行，但两路
/// 订阅者的调度顺序非确定，故 arm 前轮询确认子 session 已在注册表）。
const OOPIF_SESSION_REGISTER_TIMEOUT: Duration = Duration::from_secs(5);
const LATE_TARGET_DRAIN_MAX_PASSES: usize = 4;

/// **OOPIF arm 后台循环（接线骨架）**：订阅 `Target.attachedToTarget`（全 session 通配），
/// 对类型为 **`iframe`** 且**非本 page session** 的子 session arm 一个 [`InjectionManager`]
/// 入 `oopif_managers`。这让跨进程 OOPIF 子帧也能各自物化 utility world、跑 aria 注入。
///
/// **裁决⑥：只收 `type=="iframe"`，绝不收 `page`**（type 分流经 [`crate::tabs::should_arm_as_oopif`]）。
/// 本循环订阅的是**全局** attach 事件；若放行 `type=="page"`，看到**兄弟顶层 tab**（另一 page，
/// sid≠自己）会把它 arm 进自己的 `oopif_managers`，致 observe 活动 tab 时把兄弟整页内容当 OOPIF 子帧
/// 拼进来（**跨标签污染**）。顶层 page 归 tab 发现循环（[`spawn_tab_discovery_loop`] /
/// [`crate::tabs::should_arm_as_page`]）；二者各自筛 type，**严格互补、互不重叠**。
///
/// **现实（TODO(verify-oopif)）**：真 OOPIF 需跨源 http origin 才另起 `type=="iframe"` 子 session；
/// `file://` srcdoc/同源 iframe 是**同进程**（不另起子 session），故离线 fixture 触发不了这条路径。
/// 本循环是架构接线，真跨源路由须 http fixture / 真页后续验。所有错误 best-effort：单个子 session arm
/// 失败只 warn 不影响其它，绝不 panic。
fn spawn_oopif_arm_loop(
    conn: Connection,
    page_session: String,
    oopif_managers: std::sync::Arc<Mutex<HashMap<String, OopifEntry>>>,
) -> tokio::task::JoinHandle<()> {
    // OOPIF attach/detach is lifecycle authority, not telemetry. Reliable
    // subscribers are bounded by entry + byte budgets in SessionRegistry; any
    // overflow poisons the connection, whose Host fatal supervisor owns exact
    // process-tree cleanup. A lossy broadcast receiver could silently miss the
    // one target which bypasses MAX_OOPIFS_PER_TAB.
    let mut attached_rx = conn.subscribe_reliable(EventAttachedToTarget::IDENTIFIER, None);
    let mut detached_rx = conn.subscribe_reliable(EventDetachedFromTarget::IDENTIFIER, None);
    let mut destroyed_rx = conn.subscribe_reliable(EventTargetDestroyed::IDENTIFIER, None);
    let mut crashed_rx = conn.subscribe_reliable(EventTargetCrashed::IDENTIFIER, None);
    tokio::spawn(async move {
        'events: loop {
            tokio::select! {
                event = attached_rx.recv() => {
                    let Some(ev) = event else {
                        break 'events;
                    };
                    let event_parent_session = ev.session_id;
                    let Ok(attached) = serde_json::from_value::<EventAttachedToTarget>(ev.params)
                    else {
                        poison_host_control_stream(
                            &conn,
                            "malformed Target.attachedToTarget event in OOPIF authority",
                        );
                        break 'events;
                    };
                    let sid: String = attached.session_id.into();
                    let target_id: String = attached.target_info.target_id.into();
                    let target_type = attached.target_info.r#type;
                    let identifiers_valid =
                        crate::session::validate_cdp_identifier("OOPIF session id", &sid)
                            .and_then(|()| {
                                crate::session::validate_cdp_identifier(
                                    "OOPIF target id",
                                    &target_id,
                                )
                            })
                            .and_then(|()| {
                                (!event_parent_session.is_empty())
                                    .then_some(event_parent_session.as_str())
                                    .map_or(Ok(()), |parent| {
                                        crate::session::validate_cdp_identifier(
                                            "OOPIF parent session id",
                                            parent,
                                        )
                                    })
                            });
                    if let Err(error) = identifiers_valid {
                        conn.registry().poison_connection(error);
                        break 'events;
                    }

                    let is_own_page_session = sid == page_session;
                    let (already_armed, at_capacity) = {
                        let managers = oopif_managers
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        (
                            managers.contains_key(&sid),
                            !oopif_capacity_available(managers.len()),
                        )
                    };
                    if !crate::tabs::should_arm_as_oopif_for_parent(
                        &target_type,
                        &event_parent_session,
                        &page_session,
                        is_own_page_session,
                        already_armed,
                    ) {
                        continue;
                    }
                    if at_capacity {
                        tracing::warn!(
                            target: "nomi_browser_engine::backend::cdp",
                            target_id_suffix = %cdp_id_suffix(&target_id),
                            limit = MAX_OOPIFS_PER_TAB,
                            "per-tab OOPIF capacity exhausted; closing excess target"
                        );
                        if let Err(error) = close_target_or_confirm_absent(&conn, &target_id).await {
                            poison_host_control_stream(
                                &conn,
                                format!(
                                    "excess OOPIF exact cleanup failed; Host cleanup required: {error}"
                                ),
                            );
                            break 'events;
                        }
                        continue;
                    }

                    let deadline =
                        tokio::time::Instant::now() + OOPIF_SESSION_REGISTER_TIMEOUT;
                    let registry = conn.registry().clone();
                    while !registry.has_session(&sid) {
                        if tokio::time::Instant::now() >= deadline {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    if !registry.has_session(&sid) {
                        // An unregistered but live renderer cannot be omitted
                        // from the structural count. Close its exact target or
                        // escalate to whole-Host cleanup.
                        if let Err(error) = close_target_or_confirm_absent(&conn, &target_id).await {
                            poison_host_control_stream(
                                &conn,
                                format!(
                                    "unregistered OOPIF exact cleanup failed; Host cleanup required: {error}"
                                ),
                            );
                            break 'events;
                        }
                        continue;
                    }

                    let manager = InjectionManager::new(conn.clone(), sid.clone());
                    match manager.arm().await {
                        Ok(loop_handle) => {
                            oopif_managers
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .insert(
                                    sid.clone(),
                                    OopifEntry {
                                        target_id,
                                        manager,
                                        _loop: loop_handle,
                                    },
                                );
                            tracing::debug!(
                                target: "nomi_browser_engine::backend::cdp",
                                session_id = %sid, target_type = %target_type,
                                "armed OOPIF child session injection (TODO(verify-oopif))"
                            );
                        }
                        Err(error) => {
                            // Failed arming must not leave an uncharged renderer
                            // which a later attach burst can use to bypass the
                            // manager cap.
                            if let Err(cleanup_error) =
                                close_target_or_confirm_absent(&conn, &target_id).await
                            {
                                poison_host_control_stream(
                                    &conn,
                                    format!(
                                        "failed OOPIF arm ({error}) and exact cleanup ({cleanup_error})"
                                    ),
                                );
                                break 'events;
                            }
                        }
                    }
                }
                event = detached_rx.recv() => {
                    let Some(ev) = event else {
                        break 'events;
                    };
                    let Ok(detached) =
                        serde_json::from_value::<EventDetachedFromTarget>(ev.params)
                    else {
                        poison_host_control_stream(
                            &conn,
                            "malformed Target.detachedFromTarget event in OOPIF authority",
                        );
                        break 'events;
                    };
                    let sid: String = detached.session_id.into();
                    if let Err(error) =
                        crate::session::validate_cdp_identifier("detached OOPIF session id", &sid)
                    {
                        conn.registry().poison_connection(error);
                        break 'events;
                    }
                    remove_oopif_session(&oopif_managers, &sid);
                }
                event = destroyed_rx.recv() => {
                    let Some(ev) = event else {
                        break 'events;
                    };
                    let Ok(destroyed) =
                        serde_json::from_value::<EventTargetDestroyed>(ev.params)
                    else {
                        poison_host_control_stream(
                            &conn,
                            "malformed Target.targetDestroyed event in OOPIF authority",
                        );
                        break 'events;
                    };
                    let target_id: String = destroyed.target_id.into();
                    if let Err(error) =
                        crate::session::validate_cdp_identifier("destroyed OOPIF target id", &target_id)
                    {
                        conn.registry().poison_connection(error);
                        break 'events;
                    }
                    remove_oopif_target(&oopif_managers, &target_id);
                }
                event = crashed_rx.recv() => {
                    let Some(ev) = event else {
                        break 'events;
                    };
                    let Ok(crashed) = serde_json::from_value::<EventTargetCrashed>(ev.params)
                    else {
                        poison_host_control_stream(
                            &conn,
                            "malformed Target.targetCrashed event in OOPIF authority",
                        );
                        break 'events;
                    };
                    let target_id: String = crashed.target_id.into();
                    if let Err(error) =
                        crate::session::validate_cdp_identifier("crashed OOPIF target id", &target_id)
                    {
                        conn.registry().poison_connection(error);
                        break 'events;
                    }
                    remove_oopif_target(&oopif_managers, &target_id);
                }
            }
        }
        drain_oopif_entries(&oopif_managers);
    })
}

fn poison_host_control_stream(conn: &Connection, reason: impl Into<String>) {
    conn.registry()
        .poison_connection(TransportError::Protocol(reason.into()));
}

fn drain_oopif_entries(managers: &Arc<Mutex<HashMap<String, OopifEntry>>>) -> usize {
    let mut managers = managers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let count = managers.len();
    // `OopifEntry::drop` aborts every injection loop.
    managers.clear();
    count
}

fn remove_oopif_session(
    managers: &Arc<Mutex<HashMap<String, OopifEntry>>>,
    session_id: &str,
) -> bool {
    managers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id)
        .is_some()
}

fn remove_oopif_target(
    managers: &Arc<Mutex<HashMap<String, OopifEntry>>>,
    target_id: &str,
) -> usize {
    let mut managers = managers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = managers.len();
    managers.retain(|_, entry| entry.target_id != target_id);
    before - managers.len()
}

/// Optional per-tab request timing metadata with exact byte accounting.
///
/// Saturation drops only the new timing sample: the bounded network debug ring
/// still records the request, and page/network behavior is never affected.
/// Terminal events remove the exact request id so completed traffic cannot
/// occupy all slots indefinitely.
#[derive(Default)]
struct DebugRequestTimestamps {
    entries: HashMap<String, f64>,
    retained_key_bytes: usize,
}

impl DebugRequestTimestamps {
    fn insert(&mut self, request_id: String, timestamp: f64) -> bool {
        let key_bytes = request_id.len();
        if key_bytes > MAX_DEBUG_REQUEST_TIMESTAMP_KEY_BYTES {
            return false;
        }
        if let Some(existing) = self.entries.get_mut(&request_id) {
            *existing = timestamp;
            return true;
        }
        let Some(next_key_bytes) = self.retained_key_bytes.checked_add(key_bytes) else {
            return false;
        };
        if self.entries.len() >= MAX_DEBUG_REQUEST_TIMESTAMPS
            || next_key_bytes > MAX_DEBUG_REQUEST_TIMESTAMP_TOTAL_KEY_BYTES
        {
            return false;
        }
        self.entries.insert(request_id, timestamp);
        self.retained_key_bytes = next_key_bytes;
        true
    }

    fn remove(&mut self, request_id: &str) -> Option<f64> {
        let removed = self.entries.remove(request_id)?;
        self.retained_key_bytes = self.retained_key_bytes.saturating_sub(request_id.len());
        Some(removed)
    }

    #[cfg(test)]
    fn retained_counts(&self) -> (usize, usize) {
        (self.entries.len(), self.retained_key_bytes)
    }
}

/// **调试捕获后台循环**：启用 `Runtime.enable` + `Log.enable`（`Network.enable` 由 navigate 幂等
/// 调用，这里也 enable 确保非导航期网络事件亦被捕获），然后订阅事件流（长驻）写入 per-tab
/// [`crate::debug_capture::DebugBuffers`]。循环随连接关闭或 `.abort()` 终止。
///
/// 事件处理纯被动观察（**无** `Fetch.enable`/`requestPaused` 拦截——出口防火墙通道不碰）。
#[allow(clippy::collapsible_if)]
fn spawn_debug_capture_loop(
    conn: Connection,
    session_id: String,
    buffers: std::sync::Arc<std::sync::Mutex<crate::debug_capture::DebugBuffers>>,
) -> tokio::task::JoinHandle<()> {
    use chromiumoxide::cdp::js_protocol::runtime::EnableParams as RuntimeEnableParams;
    use chromiumoxide::cdp::browser_protocol::log::EnableParams as LogEnableParams;

    tokio::spawn(async move {
        // enable Runtime + Log（best-effort：失败仅 warn，不阻断整个 tab）。
        // Network.enable 由 navigate 路径已幂等调用；此处也 enable 兜底。
        let _ = conn
            .send::<RuntimeEnableParams>(&session_id, &RuntimeEnableParams::default())
            .await;
        let _ = conn
            .send::<LogEnableParams>(&session_id, &LogEnableParams::default())
            .await;
        let _ = conn
            .send::<NetworkEnableParams>(&session_id, &NetworkEnableParams::default())
            .await;

        // 订阅全部相关事件。
        let mut console_rx = conn.subscribe("Runtime.consoleAPICalled", Some(&session_id));
        let mut exception_rx = conn.subscribe("Runtime.exceptionThrown", Some(&session_id));
        let mut log_rx = conn.subscribe("Log.entryAdded", Some(&session_id));
        let mut req_rx = conn.subscribe("Network.requestWillBeSent", Some(&session_id));
        let mut resp_rx = conn.subscribe("Network.responseReceived", Some(&session_id));
        let mut fin_rx = conn.subscribe("Network.loadingFinished", Some(&session_id));
        let mut fail_rx = conn.subscribe("Network.loadingFailed", Some(&session_id));

        // 内部 requestId → 时间戳映射（用于算 duration_ms），按条数和 key bytes 双重有界。
        let mut request_timestamps = DebugRequestTimestamps::default();

        loop {
            tokio::select! {
                biased;
                ev = console_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            if let Some(entry) = crate::debug_capture::map_console_event(&e.params) {
                                if let Ok(mut b) = buffers.lock() {
                                    b.console.push(entry);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                ev = exception_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            if let Some(entry) = crate::debug_capture::map_exception_event(&e.params) {
                                if let Ok(mut b) = buffers.lock() {
                                    b.errors.push(entry);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                ev = log_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            if let Some(entry) = crate::debug_capture::map_log_error_event(&e.params) {
                                if let Ok(mut b) = buffers.lock() {
                                    b.errors.push(entry);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                ev = req_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            if let Some((id, entry)) = crate::debug_capture::map_request_will_be_sent(&e.params) {
                                // 记录 requestId 时间戳用于后续算 duration。
                                let ts = e.params.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                // Timing is optional debug metadata. Saturation drops this
                                // sample without clearing unrelated in-flight requests.
                                let _ = request_timestamps.insert(id, ts);
                                if let Ok(mut b) = buffers.lock() {
                                    b.network.push(entry);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                ev = resp_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            if let Some(_request_id) = e.params.get("requestId").and_then(|v| v.as_str()) {
                                if let Ok(mut b) = buffers.lock() {
                                    // 找到对应的 NetworkEntry 补全 response 信息（遍历 ring 尾部）。
                                    for entry in b.network.iter_mut() {
                                        if entry.url.is_empty() { continue; }
                                        // 按 URL+method 找不太可靠，用最近匹配（ring 尾 = 最新）。
                                        // 改进：在 entry 上存 request_id。暂且 patch 最后一个匹配的 pending entry。
                                    }
                                    // 简化实现：patch 最后一个 status==None 的 entry（近似匹配）。
                                    if let Some(last_pending) = b.network.iter_mut().rev().find(|e| e.status.is_none() && !e.failed) {
                                        crate::debug_capture::patch_response_received(last_pending, &e.params);
                                    }
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                ev = fin_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            let req_id = e.params.get("requestId").and_then(|v| v.as_str()).unwrap_or("");
                            let req_ts = request_timestamps.remove(req_id).unwrap_or(0.0);
                            if let Ok(mut b) = buffers.lock() {
                                if let Some(entry) = b.network.iter_mut().rev().find(|e| e.duration_ms.is_none() && !e.failed && e.status.is_some()) {
                                    crate::debug_capture::patch_loading_finished(entry, &e.params, req_ts);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                ev = fail_rx.recv() => {
                    match ev {
                        Ok(e) => {
                            if let Some(req_id) = e.params.get("requestId").and_then(|v| v.as_str()) {
                                let _ = request_timestamps.remove(req_id);
                            }
                            if let Ok(mut b) = buffers.lock() {
                                if let Some(entry) = b.network.iter_mut().rev().find(|e| !e.failed && e.status.is_none()) {
                                    crate::debug_capture::patch_loading_failed(entry, &e.params);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    })
}

fn bounded_rendered_html_expression() -> String {
    let content_limit = crate::engine::MAX_RENDERED_HTML_BYTES
        .saturating_sub(crate::engine::RENDERED_HTML_TRUNCATION_MARKER.len());
    format!(
        "(() => {{ try {{ \
           const text = document.documentElement ? document.documentElement.outerHTML : ''; \
           const limit = {content_limit}; \
           let bytes = 0; let end = 0; \
           while (end < text.length) {{ \
             const cp = text.codePointAt(end); \
             const width = cp <= 0x7f ? 1 : cp <= 0x7ff ? 2 : cp <= 0xffff ? 3 : 4; \
             const units = cp > 0xffff ? 2 : 1; \
             if (bytes + width > limit) break; \
             bytes += width; end += units; \
           }} \
           return {{ text: end === text.length ? text : text.slice(0, end), \
                     truncated: end < text.length, retainedUtf8Bytes: bytes }}; \
         }} catch (e) {{ return {{ text: '', truncated: false, retainedUtf8Bytes: 0 }}; }} }})()"
    )
}

fn rendered_html_from_renderer_value(value: &serde_json::Value) -> String {
    let raw = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.as_str())
        .unwrap_or_default();
    let renderer_truncated = value
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let marker = crate::engine::RENDERED_HTML_TRUNCATION_MARKER;
    let content_limit = crate::engine::MAX_RENDERED_HTML_BYTES.saturating_sub(marker.len());
    let bounded = crate::actions::utf8_prefix_at_most(raw, content_limit);
    let truncated = renderer_truncated || bounded.len() < raw.len();
    let mut html = String::with_capacity(
        bounded
            .len()
            .saturating_add(if truncated { marker.len() } else { 0 }),
    );
    html.push_str(bounded);
    if truncated {
        html.push_str(marker);
    }
    html
}

#[async_trait]
impl BrowserEngine for CdpBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            browser_ready: true,
            headful: self.headful,
            display_available: self.display_available,
            engine: "chromium".into(),
        }
    }

    async fn navigate(&self, url: &str, _new_tab: bool) -> Result<NavResult, BrowserError> {
        // observe⊥act：整段导航持引擎 op_mutex（DESIGN §22），不与在途 observe/act 在本引擎交错。
        let _op = tokio::select! {
            guard = self.op_mutex.lock() => guard,
            _ = self.lane_cancel.cancelled() => return Err(BrowserError::TargetClosed),
        };
        // D1/D2：在 active tab 的 page session 上原地导航（多 tab 路由 / new_tab 由 D3 实现）。短暂
        // 锁 tabs 克隆出 active 句柄后立即释放（不跨 await 持 tabs 锁）。
        let handles = self.active_tab_handles().await?;
        let session = handles.session_id.as_str();
        let main_frame_id = handles.main_frame_id.clone();
        tokio::select! {
            result = self.navigate_on_session(session, &main_frame_id, url) => result,
            _ = self.lane_cancel.cancelled() => Err(BrowserError::TargetClosed),
        }
    }

    async fn screenshot(&self) -> Result<Vec<u8>, BrowserError> {
        // observe⊥act：持 op_mutex（DESIGN §22），截图不与在途 act 改 DOM 交错。
        let _op = tokio::select! {
            guard = self.op_mutex.lock() => guard,
            _ = self.lane_cancel.cancelled() => return Err(BrowserError::TargetClosed),
        };
        // D1：截 active tab。
        let session = self.active_tab_handles().await?.session_id;
        let params = CaptureScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .build();

        let result = tokio::select! {
            result = self.conn.send::<CaptureScreenshotParams>(&session, &params) => {
                result.map_err(map_transport_err)?
            }
            _ = self.lane_cancel.cancelled() => return Err(BrowserError::TargetClosed),
        };

        let shot: CaptureScreenshotReturns = serde_json::from_value(result.clone()).map_err(|e| {
            BrowserError::Other(format!("parse captureScreenshot response: {e}"))
        })?;
        // `data` 是 base64。用 chromiumoxide_types::Binary 的 AsRef<str> 取串后 decode。
        let b64: &str = shot.data.as_ref();
        decode_base64(b64).ok_or_else(|| {
            BrowserError::Other("captureScreenshot returned non-base64 data".into())
        })
    }

    async fn observe(&self, opts: &ObserveOpts) -> Result<Observation, BrowserError> {
        // observe⊥act：整段快照持 op_mutex，动作不可在序列化中途改 DOM（否则交回模型陈旧 ref）。
        let _op = tokio::select! {
            guard = self.op_mutex.lock() => guard,
            _ = self.lane_cancel.cancelled() => return Err(BrowserError::TargetClosed),
        };
        tokio::select! {
            result = self.observe_impl(opts) => result,
            _ = self.lane_cancel.cancelled() => Err(BrowserError::TargetClosed),
        }
    }

    async fn rendered_html(&self) -> Result<String, BrowserError> {
        // NOTE: 故意**不**持 op_mutex——只读 DOM 序列化（知识库管线消费），不得阻塞在途 act；其调用链
        // （active_frame_eval→active_page_frame→conn.send）不触碰被包裹的四个 trait 方法,无重入风险。
        // Read the **post-JS** DOM as raw HTML on the active frame. Read-only
        // (no redaction / no `<data>` wrap — see the trait doc): the knowledge
        // layer runs this through its own HTML→markdown pipeline, so it must get
        // un-transformed markup. The renderer returns only a bounded UTF-8
        // prefix rather than letting `outerHTML` inflate a by-value CDP message
        // up to the transport's 64 MiB emergency limit. Rust validates the
        // boundary again before retaining the string.
        let expression = bounded_rendered_html_expression();
        let value = self.active_frame_eval(&expression).await?;
        Ok(rendered_html_from_renderer_value(&value))
    }

    async fn act(
        &self,
        spec: &crate::actions::ActSpec,
        progress: &crate::progress::Progress,
    ) -> Result<crate::actions::ActResult, BrowserError> {
        // observe⊥act + per-target act 串行：一引擎一次一动作。受 progress 截止时间界定,op_mutex 不无界持有。
        let _op = tokio::select! {
            guard = self.op_mutex.lock() => guard,
            _ = self.lane_cancel.cancelled() => return Err(BrowserError::TargetClosed),
        };
        // C1：Click/Type/SetValue 经 act_impl 串 B2-B6 执行；其它 ActSpec 仍 Unsupported（C2/C3/D/E/F）。
        tokio::select! {
            result = self.act_impl(spec, progress) => result,
            _ = self.lane_cancel.cancelled() => Err(BrowserError::TargetClosed),
        }
    }

    async fn debug_snapshot(
        &self,
    ) -> Result<crate::debug_capture::DebugSnapshot, BrowserError> {
        let handles = self.active_tab_handles().await?;
        Ok(crate::debug_capture::DebugSnapshot::from_buffers(&handles.debug))
    }

    async fn bring_to_front(&self) -> Result<(), BrowserError> {
        CdpBackend::bring_to_front(self).await
    }

    async fn capture_cookie_state(
        &self,
    ) -> Result<crate::storage_state::StorageState, BrowserError> {
        CdpBackend::capture_cookies(self).await
    }

    async fn capture_storage_state(
        &self,
    ) -> Result<crate::storage_state::StorageState, BrowserError> {
        CdpBackend::capture_storage_state(self).await
    }

    async fn tabs(&self) -> Result<Vec<BrowserTabInfo>, BrowserError> {
        self.structured_tab_inventory().await
    }

    async fn click_at_css_point(&self, x: f64, y: f64) -> Result<(), BrowserError> {
        use crate::input::Point;
        self.click_at(Point { x, y }).await
    }

    async fn device_pixel_ratio(&self) -> Result<f64, BrowserError> {
        // Query the active page's window.devicePixelRatio (P7B: visual-fallback coord mapping).
        // Best-effort: any exception / non-positive / non-number → fall back to 1.0 (never block
        // a visual-fallback click on a DPR probe; 1.0 is correct for headless anyway).
        let session = self.active_tab_handles().await?.session_id;
        let mut params = EvaluateParams::new("window.devicePixelRatio".to_string());
        params.return_by_value = Some(true);
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        let dpr = result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(serde_json::Value::as_f64)
            .filter(|d| *d > 0.0)
            .unwrap_or(1.0);
        Ok(dpr)
    }
}

impl CdpBackend {
    /// **detach/crash 事件源 → `Progress::abort` 接线**（B6，DESIGN §11/§22）：在一次 `act` 期间
    /// **临时订阅** `Target.detachedFromTarget` / `Target.targetCrashed`（page target 没了/崩了）
    /// 与 `Page.frameDetached`（动作所在帧从树上 detach），把它们映射到对当前动作 [`Progress`] 的
    /// `abort(PageClosed|FrameDetached)`，使进行中的动作（在 [`crate::actions::run_act_with_retry`]
    /// 的 `progress.race` 上）**立即取消**（远早于 deadline），而非白等超时。
    ///
    /// 形态（**最小可行**，C1 复用）：
    /// 1. 据传入的 `parent`（动作的总 deadline/取消上下文）派生一个**子** [`Progress`]（共享 timeout +
    ///    token 层级：parent 取消 → 子立即取消）。动作跑在返回的子 Progress 上。
    /// 2. spawn 一个监听任务，select 三路事件订阅；命中即对子 Progress `abort`：
    ///    - `Target.detachedFromTarget`（params.sessionId == 本 page session）
    ///      → `abort(PageClosed)`；
    ///    - `Target.targetCrashed`（params.targetId == 本 page target）
    ///      → `abort(TargetCrashed)`；
    ///    - `Page.frameDetached`（params.frameId == `frame_id`，即动作所在帧）→ `abort(FrameDetached)`。
    /// 3. 返回 `(child, guard)`：动作在 `child` 上跑；`guard` 持监听任务句柄，**Drop 即取消监听**
    ///    （动作结束——成功/失败均然——guard 离开作用域，临时订阅随之收摊）。
    ///
    /// **绝不 panic**：监听任务里所有解析失败 best-effort（continue）；订阅通道关闭/落后按 broadcast
    /// 语义处理（closed→退出、lagged→继续）。close 与 crash 保留不同的 typed error，同时都允许
    /// 后续操作走 active-tab survivor/replacement 恢复。
    ///
    /// **D1：async**——内部短暂锁 tabs 取 active tab 的 page session/target（订阅
    /// Page.frameDetached 限定它 + 分别比对 detach sessionId / crash targetId）后立即释放。
    /// active tab 缺失时会先走 survivor/replacement 恢复；失败返 Err（绝不 panic）。
    pub async fn arm_act_abort(
        &self,
        parent: &Progress,
        frame_id: &str,
    ) -> Result<(Arc<Progress>, ActAbortGuard), BrowserError> {
        // D1：同时取 page session 和 target id。detachedFromTarget 按
        // sessionId 路由；targetCrashed 的 CDP payload 只有 targetId。
        let page = self.active_tab_handles().await?;
        let page_session = page.session_id;
        let page_target = page.target_id;

        // 子 Progress：共享 parent 的剩余 deadline（保守用 parent 当下剩余预算的近似——这里直接复用
        // parent 的 token 层级，timeout 取一个不短于 parent 的值；act 的真实 deadline 由调用方在 parent
        // 上设定，子继承其取消）。用 child(timeout, parent_token) 建层级：parent 取消 → 子立即取消。
        let child = Arc::new(Progress::child(parent.timeout(), parent.token()));

        // 三路临时订阅（act 期间有效，guard drop 后监听任务被 abort，订阅 Receiver 随任务 drop 收摊）。
        let mut detached_rx = self
            .conn
            .subscribe_reliable_for_task(
                "Target.detachedFromTarget",
                None,
                &self.reliable_event_task_budget,
            )
            .map_err(map_transport_err)?;
        let mut crashed_rx = self
            .conn
            .subscribe_reliable_for_task(
                "Target.targetCrashed",
                None,
                &self.reliable_event_task_budget,
            )
            .map_err(map_transport_err)?;
        let mut frame_detached_rx = self
            .conn
            .subscribe("Page.frameDetached", Some(&page_session));

        let watch_frame = frame_id.to_string();
        let child_for_task = Arc::clone(&child);

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // page target detach（tab 关闭）：params.sessionId == 本 page session → PageClosed。
                    ev = detached_rx.recv() => match ev {
                        Some(ev) => {
                            if event_session_matches(&ev.params, &page_session) {
                                child_for_task.abort(AbortReason::PageClosed);
                                break;
                            }
                        }
                        None => break,
                    },
                    // targetCrashed 是 root event，按 targetId 命中并保留 crash taxonomy。
                    ev = crashed_rx.recv() => match ev {
                        Some(ev) => {
                            if event_target_matches(&ev.params, &page_target) {
                                child_for_task.abort(AbortReason::TargetCrashed);
                                break;
                            }
                        }
                        None => break,
                    },
                    // 动作所在帧 detach：params.frameId == watch_frame → FrameDetached。
                    ev = frame_detached_rx.recv() => match ev {
                        Ok(ev) => {
                            if event_frame_matches(&ev.params, &watch_frame) {
                                child_for_task.abort(AbortReason::FrameDetached);
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                }
            }
        });

        Ok((child, ActAbortGuard { handle: Some(handle) }))
    }

    /// 查当前 page 的 url：`Page.getNavigationHistory` 取 currentIndex 指向的 entry url。
    /// 失败/取不到返回 None（调用方回退）。
    async fn current_url(&self, session: &str) -> Option<String> {
        use chromiumoxide::cdp::browser_protocol::page::GetNavigationHistoryParams;
        let result = self
            .conn
            .send::<GetNavigationHistoryParams>(session, &GetNavigationHistoryParams::default())
            .await
            .ok()?;
        let idx = result.get("currentIndex")?.as_i64()?;
        let entries = result.get("entries")?.as_array()?;
        let entry = entries.get(usize::try_from(idx).ok()?)?;
        entry.get("url")?.as_str().map(|s| s.to_string())
    }

    /// SD-4：一次 `Page.getNavigationHistory` 同时取 url + POST 标志（`observe_impl` 用）。
    /// 失败 → `(None, false)`（保守：不误判普通页 reload 为不可逆）。
    async fn url_and_post_flag(&self, session: &str) -> (Option<String>, bool) {
        use chromiumoxide::cdp::browser_protocol::page::GetNavigationHistoryParams;
        let result = match self
            .conn
            .send::<GetNavigationHistoryParams>(session, &GetNavigationHistoryParams::default())
            .await
        {
            Ok(v) => v,
            Err(_) => return (None, false),
        };
        let current_index = match result.get("currentIndex").and_then(|v| v.as_i64()) {
            Some(idx) => idx,
            None => return (None, false),
        };
        let entries_val = match result.get("entries") {
            Some(v) => v.clone(),
            None => return (None, false),
        };
        let url = entries_val
            .as_array()
            .and_then(|arr| usize::try_from(current_index).ok().and_then(|i| arr.get(i)))
            .and_then(|e| e.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());
        let is_post = nav::current_entry_is_post(&entries_val, current_index);
        (url, is_post)
    }

    /// **navigate settle 全链**（D2，DESIGN §12 + 裁决⑤）：在给定 page session 上原地导航，跑成熟
    /// 的生命周期判定后返回 [`NavResult`]。D3 的 new_tab/switch 走别的 session，本方法只管「一个
    /// session 上的一次导航」。
    ///
    /// 时序（先订阅后导航，避免快页面在订阅前就 load 完）：
    /// 1. `Page.enable` + `Network.enable`（生命周期 + 网络事件前置；幂等）。
    /// 2. **先订阅** 四路事件（全在本 session 上）：`domContentEventFired` / `loadEventFired` /
    ///    `navigatedWithinDocument`（SPA）/ `requestWillBeSent` / `responseReceived` /
    ///    `loadingFinished` / `loadingFailed`。
    /// 3. `Page.navigate`（errorText 即导航失败 → `NavFailed`）。
    /// 4. **settle 阶梯**（[`Self::run_settle`]）：等 DOMContentLoaded → 短 settle → 升级 Load；
    ///    其间 SPA 软导航信号 → 走软导航降级路径（不重新等 load）。
    /// 5. **networkidle 独立短 cap**（[`Self::wait_network_idle`]）：仅在已达 Load 后做；inflight 持续
    ///    0 满 500ms → `NetworkIdle`；到 4s cap 仍未达成（长轮询站）→ 降级回 `Load`。**绝不并入**
    ///    导航总超时。
    /// 6. http_status 从主帧 Document `responseReceived` 取；final_url 查 history；redirected 用
    ///    [`nav::is_redirect`] 归一化比较（**非裸 `!=`**）。
    ///
    /// 良性态不报错：networkidle cap 降级 / SPA 软导航 / 302 重定向都 `Ok`（success 语义在 facade）。
    async fn navigate_on_session(
        &self,
        session: &str,
        main_frame_id: &str,
        url: &str,
    ) -> Result<NavResult, BrowserError> {
        // navigate 的触发 = `Page.navigate`（errorText 即导航失败 → NavFailed）。settle 全链复用
        // [`Self::settle_after_trigger`]（D2 母本；back/forward/reload 同样复用它，零分叉）。
        self.settle_after_trigger(session, main_frame_id, url, |conn, session| async move {
            let nav_params = NavigateParams::new(url);
            let nav_result = conn
                .send::<NavigateParams>(&session, &nav_params)
                .await
                .map_err(map_transport_err)?;
            let nav: NavigateReturns = serde_json::from_value(nav_result.clone()).map_err(|e| {
                BrowserError::Other(format!("parse Page.navigate response: {e} (raw={nav_result})"))
            })?;
            if let Some(err_text) = nav.error_text.as_deref() {
                return Err(BrowserError::NavFailed {
                    kind: err_text.to_string(),
                });
            }
            Ok(())
        })
        .await
    }

    /// **导航 settle 母本（D2 抽出，D4 复用）**：在 `session` 上**先订阅**全部生命周期/网络事件，
    /// 再跑 `trigger`（发那条引发导航的 CDP 命令——`Page.navigate` / `navigateToHistoryEntry` /
    /// `Page.reload`），随后跑成熟的 settle 阶梯（[`Self::run_settle`] + networkidle 短 cap）并返回
    /// [`NavResult`]。**back/forward/reload 与 navigate 共用本方法**——它们只在 `trigger` 不同
    /// （settle 逻辑零分叉，对齐 D4「settle 复用 D2 run_settle」）。
    ///
    /// 时序与 D2 一致（见 [`Self::navigate_on_session`] 旧 doc）：enable → 订阅 7 路事件 → trigger →
    /// run_settle（DCL→短 settle→Load；SPA 软导航降级）→ networkidle 独立短 cap → final_url +
    /// redirected（URL-normalize 比较）+ http_status（主帧 Document responseReceived）。
    ///
    /// `expected_url` 用于 redirect 归一化比较的「from」端：navigate 传请求 url；reload/history 导航传
    /// 触发**前**的当前 url（reload 通常不变 → 不算 redirect；history 导航回到的 entry url 即「目标」，
    /// 与 final_url 同源 → 不算 redirect）。良性态（cap 降级 / SPA / 302）皆 `Ok`。
    async fn settle_after_trigger<F, Fut>(
        &self,
        session: &str,
        main_frame_id: &str,
        expected_url: &str,
        trigger: F,
    ) -> Result<NavResult, BrowserError>
    where
        F: FnOnce(Connection, String) -> Fut,
        Fut: std::future::Future<Output = Result<(), BrowserError>>,
    {
        // 1) Page.enable + Network.enable（生命周期 + 网络事件前置；幂等）。
        self.conn
            .send::<PageEnableParams>(session, &PageEnableParams::default())
            .await
            .map_err(map_transport_err)?;
        self.conn
            .send::<NetworkEnableParams>(session, &NetworkEnableParams::default())
            .await
            .map_err(map_transport_err)?;

        // 2) 先订阅全部相关事件（trigger 之前，避免漏掉早到的 DCL/load/response）。
        let mut dcl_rx = self.conn.subscribe("Page.domContentEventFired", Some(session));
        let mut load_rx = self.conn.subscribe("Page.loadEventFired", Some(session));
        let mut spa_rx = self
            .conn
            .subscribe("Page.navigatedWithinDocument", Some(session));
        let mut response_rx = self.conn.subscribe("Network.responseReceived", Some(session));
        let mut req_rx = self.conn.subscribe("Network.requestWillBeSent", Some(session));
        let mut fin_rx = self.conn.subscribe("Network.loadingFinished", Some(session));
        let mut fail_rx = self.conn.subscribe("Network.loadingFailed", Some(session));

        let mut http_status: Option<u16> = None;
        let mut inflight = InflightCounter::new();

        // 3) 触发导航（navigate / navigateToHistoryEntry / reload）。失败 → 上抛（NavFailed/传输错）。
        trigger(self.conn.clone(), session.to_string()).await?;

        // 4) settle 阶梯（DCL → 短 settle → Load；SPA 软导航降级；记 http_status + inflight）。
        let settle = self
            .run_settle(
                main_frame_id,
                &mut dcl_rx,
                &mut load_rx,
                &mut spa_rx,
                &mut response_rx,
                &mut req_rx,
                &mut fin_rx,
                &mut fail_rx,
                &mut http_status,
                &mut inflight,
            )
            .await;

        // 5) 决定 load_state（SPA 软导航 / Load 后 networkidle 短 cap / 降级——与 D2 一致）。
        let base_state = match settle.state {
            NavSettleState::Load => LoadState::Load,
            NavSettleState::DomContentLoaded => LoadState::DomContentLoaded,
            NavSettleState::Commit => LoadState::Commit,
        };
        let load_state = if settle.soft_nav {
            base_state
        } else if base_state == LoadState::Load {
            self.wait_network_idle(&mut inflight, &mut req_rx, &mut fin_rx, &mut fail_rx)
                .await
        } else {
            base_state
        };

        // 6) final_url + redirected（归一化比较，非裸 !=）+ http_status。
        let final_url = self
            .current_url(session)
            .await
            .unwrap_or_else(|| expected_url.to_string());
        let redirected = nav::is_redirect(expected_url, &final_url);

        Ok(NavResult {
            final_url,
            http_status,
            redirected,
            load_state,
        })
    }

    /// **settle 阶梯执行**（[`Self::navigate_on_session`] step 4）：等 DOMContentLoaded → 短 settle →
    /// Load，其间持续吸收主帧 Document responseReceived（填 http_status）+ inflight 事件（喂计数器）。
    ///
    /// 各阶段都有自己的短上限（不依赖单一大超时；总预算另由传输层每命令 30s + 这些上限兜底）：
    /// - 等 DCL：[`nav::DOMCONTENTLOADED_TIMEOUT`]（30s 上限，超时不致命 → 停在 Commit）。
    /// - 收到 DCL 后短 settle：[`nav::SETTLE_QUIET`]（100ms，给同步脚本/首批微任务喘息）。
    /// - 短 settle 后等 Load：剩余预算（载入子资源；超时不致命 → 停在 DomContentLoaded）。
    ///
    /// **SPA 软导航**：任何阶段收到 `navigatedWithinDocument` → 走 [`Self::wait_spa_soft_nav`]
    /// 降级（等 URL 变 / 短稳定），置 `soft_nav=true` 立即返回（不重新等 load）。
    #[allow(clippy::too_many_arguments)]
    async fn run_settle(
        &self,
        main_frame_id: &str,
        dcl_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        load_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        spa_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        response_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        req_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        fin_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        fail_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        http_status: &mut Option<u16>,
        inflight: &mut InflightCounter,
    ) -> SettleOutcome {
        let mut state = NavSettleState::Commit;

        // ── 阶段 1：等 DOMContentLoaded（其间吸收 response/inflight/SPA 信号）──
        let dcl_deadline = tokio::time::Instant::now() + nav::DOMCONTENTLOADED_TIMEOUT;
        loop {
            let remaining = dcl_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                // DCL 超时：停在 Commit（文档已提交但 DOM 没等到——慢站；良性，不报错）。
                return SettleOutcome { state, soft_nav: false };
            }
            tokio::select! {
                biased;
                // SPA 软导航优先：收到即降级路径，不再等 load。
                ev = spa_rx.recv() => {
                    if recv_ok(ev) {
                        self.wait_spa_soft_nav().await;
                        return SettleOutcome { state, soft_nav: true };
                    }
                }
                ev = dcl_rx.recv() => {
                    if recv_ok(ev) {
                        state = nav::advance_settle(state, LifecycleSignal::DomContentLoaded);
                        break;
                    }
                }
                // load 可能先于 DCL 订阅被处理到（极快页面）→ 直接拔高到 Load 并跳出。
                ev = load_rx.recv() => {
                    if recv_ok(ev) {
                        state = nav::advance_settle(state, LifecycleSignal::Load);
                        // 已 Load 必已过 DCL；直接进短 settle 后返回。
                        tokio::time::sleep(nav::SETTLE_QUIET).await;
                        return SettleOutcome { state, soft_nav: false };
                    }
                }
                ev = response_rx.recv() => { absorb_response(ev, main_frame_id, http_status); }
                ev = req_rx.recv() => { absorb_request(ev, inflight); }
                ev = fin_rx.recv() => { absorb_finish(ev, inflight); }
                ev = fail_rx.recv() => { absorb_fail(ev, inflight); }
                () = tokio::time::sleep(remaining) => {
                    return SettleOutcome { state, soft_nav: false };
                }
            }
        }

        // ── 阶段 2：短 settle（给同步脚本/首批微任务喘息；其间仍吸收事件）──
        let settle_deadline = tokio::time::Instant::now() + SETTLE_QUIET;
        loop {
            let remaining = settle_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                biased;
                ev = spa_rx.recv() => {
                    if recv_ok(ev) {
                        self.wait_spa_soft_nav().await;
                        return SettleOutcome { state, soft_nav: true };
                    }
                }
                ev = load_rx.recv() => {
                    if recv_ok(ev) {
                        state = nav::advance_settle(state, LifecycleSignal::Load);
                        return SettleOutcome { state, soft_nav: false };
                    }
                }
                ev = response_rx.recv() => { absorb_response(ev, main_frame_id, http_status); }
                ev = req_rx.recv() => { absorb_request(ev, inflight); }
                ev = fin_rx.recv() => { absorb_finish(ev, inflight); }
                ev = fail_rx.recv() => { absorb_fail(ev, inflight); }
                () = tokio::time::sleep(remaining) => { break; }
            }
        }

        // ── 阶段 3：等 Load（载入子资源；超时不致命 → 停在 DomContentLoaded）──
        // 可交互探测：到此 DOM 已构建（DCL 过）+ 短 settle 已过——「可交互元素出现」对应 DOM 就绪，
        // observe 链会在调用方真正反查元素时把关；这里用「等 load / 短上限内」作为「可交互稳态」近似。
        let load_deadline = tokio::time::Instant::now() + nav::DOMCONTENTLOADED_TIMEOUT;
        loop {
            let remaining = load_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                biased;
                ev = spa_rx.recv() => {
                    if recv_ok(ev) {
                        self.wait_spa_soft_nav().await;
                        return SettleOutcome { state, soft_nav: true };
                    }
                }
                ev = load_rx.recv() => {
                    if recv_ok(ev) {
                        state = nav::advance_settle(state, LifecycleSignal::Load);
                        break;
                    }
                }
                ev = response_rx.recv() => { absorb_response(ev, main_frame_id, http_status); }
                ev = req_rx.recv() => { absorb_request(ev, inflight); }
                ev = fin_rx.recv() => { absorb_finish(ev, inflight); }
                ev = fail_rx.recv() => { absorb_fail(ev, inflight); }
                () = tokio::time::sleep(remaining) => { break; }
            }
        }

        SettleOutcome { state, soft_nav: false }
    }

    /// **SPA 软导航降级**（DESIGN §12：same-document，无 newDocument → 不重新等 load）：等一个短稳定
    /// 窗口让软导航落地（URL 已由 navigatedWithinDocument 改了 history）。这里用 [`SPA_SETTLE_TIMEOUT`]
    /// 内的固定短 sleep 作为「下一目标 actionable 前的稳定点」——真正的 actionable 由调用方下一步
    /// observe/act 的 actionability 把关，本方法只兑现「软导航后不白等 load 超时」。
    async fn wait_spa_soft_nav(&self) {
        // 软导航通常瞬时；给一个远小于 SPA_SETTLE_TIMEOUT 的稳定窗口即可。封顶 SPA_SETTLE_TIMEOUT。
        let quiet = SETTLE_QUIET.min(SPA_SETTLE_TIMEOUT);
        tokio::time::sleep(quiet).await;
    }

    /// **networkidle 独立短 cap 等待**（DESIGN §12 + 裁决⑤）：在已达 Load 后调用。持续观察 inflight
    /// 事件，inflight 连续为 0 满 [`NETWORK_IDLE_QUIET`]（500ms）→ 返 `NetworkIdle`；到
    /// [`NETWORK_IDLE_CAP`]（4s）仍未达成（长轮询/SSE/WS 永不 idle）→ **降级返 `Load`**。
    ///
    /// **关键不变量**：这个 cap **完全独立**于导航总超时——4s 内拿不到 networkidle 就退而求其次返
    /// Load，绝不让长轮询站把整个 navigate 拖到 30s。良性态（cap 降级）不报错。
    async fn wait_network_idle(
        &self,
        inflight: &mut InflightCounter,
        req_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        fin_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        fail_rx: &mut tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
    ) -> LoadState {
        let cap_deadline = tokio::time::Instant::now() + NETWORK_IDLE_CAP;
        loop {
            // 当前已空闲 → 进入「连续空闲 500ms」计时；其间任何新请求 → 重新计时（回外层 loop）。
            if inflight.is_idle() {
                let quiet_until = tokio::time::Instant::now() + NETWORK_IDLE_QUIET;
                loop {
                    let now = tokio::time::Instant::now();
                    // 整段 cap 已到 → 即便正接近 quiet 也以 cap 为硬上限降级。
                    let cap_left = cap_deadline.saturating_duration_since(now);
                    let quiet_left = quiet_until.saturating_duration_since(now);
                    if quiet_left.is_zero() {
                        // 连续空闲满 500ms → networkidle 达成。
                        return LoadState::NetworkIdle;
                    }
                    if cap_left.is_zero() {
                        // cap 到（长轮询站永不 idle）→ 降级 Load（良性，不报错）。
                        return LoadState::Load;
                    }
                    // 等到 quiet 满 或 有新请求打破空闲（取较小窗口，cap 兜底）。
                    let wait = quiet_left.min(cap_left);
                    tokio::select! {
                        biased;
                        ev = req_rx.recv() => {
                            absorb_request(ev, inflight);
                            if !inflight.is_idle() { break; } // 空闲被打破 → 回外层重新等空闲
                        }
                        ev = fin_rx.recv() => { absorb_finish(ev, inflight); }
                        ev = fail_rx.recv() => { absorb_fail(ev, inflight); }
                        () = tokio::time::sleep(wait) => { /* 重新评估 quiet/cap */ }
                    }
                }
            } else {
                // 非空闲：等到空闲 或 cap 到。
                let cap_left = cap_deadline.saturating_duration_since(tokio::time::Instant::now());
                if cap_left.is_zero() {
                    return LoadState::Load;
                }
                tokio::select! {
                    biased;
                    ev = req_rx.recv() => { absorb_request(ev, inflight); }
                    ev = fin_rx.recv() => { absorb_finish(ev, inflight); }
                    ev = fail_rx.recv() => { absorb_fail(ev, inflight); }
                    () = tokio::time::sleep(cap_left) => { return LoadState::Load; }
                }
            }
        }
    }


    /// **observe 全链**（Task 6）：逐帧 `incrementalAriaSnapshot` → 缝合 → 脱敏 → 代际翻新 ref 表。
    ///
    /// **D1**：开头一次 [`Self::active_tab_handles`] 拿 active tab 句柄快照（session/injection/
    /// main_frame/oopif/ref_table），全链用它——不跨 await 持 `tabs` 锁（ref_table 是克隆出的 Arc，
    /// 锁外独立锁）。
    ///
    /// 步骤：
    /// 1. 等主帧 utility context 就绪（导航后短轮询；超时即报 NavFailed{context}）。
    /// 2. `DOM.enable`（getFrameOwner/resolveNode 的前置）。
    /// 3. 列同进程帧（frameTree）+ 续编 OOPIF 子 session，逐帧产 [`FrameSnapshot`]（seq → prefix `f<seq>`）。
    /// 4. 建 `child_frame → (parent_frame, parent_iframe_ref)` 路由（getFrameOwner→resolveNode→`_ariaRef.ref`）。
    /// 5. 自主帧起递归写入一个有硬字节上限的缓冲区，缝合成一棵树。
    /// 6. 脱敏（[`redact::redact_yaml`]）+ 不可信包裹（[`redact::wrap_untrusted`]，origin=current_url）。
    /// 7. 代际翻新：锁 active tab 的 `ref_table`，`new_generation(prev)`，解析每行 `[ref=...]` 填 entries + 表，存回。
    ///
    /// 所有 CDP/注入调用经 `map_transport_err`/`map_inject_err`，**绝不 panic**。
    pub(crate) async fn observe_impl(&self, opts: &ObserveOpts) -> Result<Observation, BrowserError> {
        // D1：一次拿 active tab 句柄快照（立即释放 tabs 锁）。全链用 handles。
        let handles = self.active_tab_handles().await?;
        let page_session = handles.session_id.clone();

        // 1) 等主帧 utility context 就绪（fresh navigate 后 world 可能还没物化）。
        self.wait_main_context_ready(&handles).await?;

        // 2) DOM.enable（iframe→子帧路由前置）。幂等。
        self.conn
            .send::<DomEnableParams>(&page_session, &DomEnableParams::default())
            .await
            .map_err(map_transport_err)?;

        // 3) 逐帧产 FrameSnapshot。frames: (seq, frame_id, session_id, snapshot)。
        let mut frames: Vec<ObservedFrame> = Vec::new();
        // Every successful frame snapshot retains one authoritative JS
        // `elements` map. Track their task-wide total while snapshotting, and
        // retain exact owners so a later-frame/parser overflow can invalidate
        // the entire unpublished generation before returning.
        let mut retained_ref_count = 0usize;
        // Distinct counters defend both sides of the trust boundary: the JS
        // counter is exact UTF-8 JSON returned by CDP; the Rust counter is the
        // heap capacity retained by deserialized FrameSnapshots.
        let mut retained_snapshot_json_bytes = 0usize;
        let mut retained_frame_bytes = 0usize;
        let mut snapshot_ref_owners: Vec<(InjectionManager, String)> = Vec::new();
        let mut next_seq: u32 = 0;
        let mut truncated = false;
        // D5：跨帧累积 password 输入的 aria ref（同帧 utility world 收集），缝合后宿主侧抹其 value。
        let mut password_refs: Vec<String> = Vec::new();
        // D5 fail-closed：任一帧 password 探测失败的标志。失败时无法精确知道哪些字段是 password，
        // 故对全部可编辑控件值整体 over-redact 兜底（绝不放行 password 明文）。
        let mut any_password_query_failed = false;
        // P7B：主帧可点击 ref 的 CSS 像素框（仅当 opts.include_boxes）。**仅主帧**（方案①）：
        // getBoundingClientRect 是帧内视口坐标，主帧 viewport 即截图坐标系；子帧需叠 iframe 偏移（方案②暂缓）。
        let mut ref_boxes: std::collections::HashMap<String, crate::engine::CssRect> =
            std::collections::HashMap::new();

        // 3a) 同进程帧（active tab page session 的 frameTree；主帧在前）。
        let same_proc_frames = handles.injection.frame_ids().await.map_err(map_inject_err)?;
        for fid in &same_proc_frames {
            match self
                .snapshot_one_frame(
                    &handles.injection,
                    fid,
                    next_seq,
                    opts,
                    retained_ref_count,
                    retained_snapshot_json_bytes,
                    retained_frame_bytes,
                )
                .await
            {
                Ok(Some((snap, frame_ref_count, frame_json_bytes, frame_heap_bytes))) => {
                    retained_ref_count = retained_ref_count.saturating_add(frame_ref_count);
                    retained_snapshot_json_bytes = retained_snapshot_json_bytes
                        .saturating_add(frame_json_bytes);
                    retained_frame_bytes = retained_frame_bytes.saturating_add(frame_heap_bytes);
                    snapshot_ref_owners.push((handles.injection.clone(), fid.clone()));
                    if frame_hit_depth_limit(&snap, opts.max_depth) {
                        truncated = true;
                    }
                    // D5：该帧 password 字段的 ref（同 utility world）。查询失败 → 置 fail-closed 标志。
                    if collect_password_refs(&handles.injection, fid, &mut password_refs).await {
                        any_password_query_failed = true;
                    }
                    // P7B：仅主帧采集可点击 ref 的 CSS 框（方案①）。紧接该帧 snapshot 之后取，确保
                    // 该帧 _lastAriaSnapshotForQuery 刚物化、未被后续帧覆盖。best-effort：失败仅 warn，
                    // 不影响 observe（拿不到框 → facade 不画 SoM、回落原始兜底）。
                    if opts.include_boxes && fid == &handles.main_frame_id {
                        match handles.injection.ref_boxes(fid).await {
                            Ok(b) => ref_boxes = b,
                            Err(e) => tracing::warn!(
                                target: "nomi_browser_engine::backend::cdp",
                                frame_id = %fid, error = ?e,
                                "ref_boxes (SoM geometry) failed for main frame (skip; visual fallback degrades to raw)"
                            ),
                        }
                    }
                    frames.push(ObservedFrame {
                        seq: next_seq,
                        frame_id: fid.clone(),
                        session_id: page_session.clone(),
                        snapshot: snap,
                    });
                    next_seq += 1;
                }
                // context 没就绪 / body 取不到 / 单帧 JS 异常：跳过该帧（best-effort，不致命）。
                Ok(None) => {}
                Err(error @ InjectError::RefCapacityExceeded { .. }) => {
                    Self::invalidate_observe_ref_state(
                        &handles.ref_table,
                        &snapshot_ref_owners,
                    )
                    .await;
                    return Err(map_inject_err(error));
                }
                Err(error @ InjectError::ObservationCapacityExceeded { .. }) => {
                    Self::invalidate_observe_ref_state(
                        &handles.ref_table,
                        &snapshot_ref_owners,
                    )
                    .await;
                    return Err(map_inject_err(error));
                }
                Err(e) => {
                    tracing::warn!(
                        target: "nomi_browser_engine::backend::cdp",
                        frame_id = %fid, error = ?e,
                        "snapshot same-process frame failed (skip)"
                    );
                }
            }
        }

        // 3b) OOPIF 子 session（跨进程子帧；接线骨架，离线 fixture 触发不到，见 TODO(verify-oopif)）。
        // 锁内**只** clone 出各 OOPIF manager 句柄（InjectionManager: Clone，共享 Arc 缓存）后立即
        // 释放锁；所有 `.await`（frame_ids / snapshot / password_refs）在锁外跑——兑现「不跨 await
        // 持锁」（避免阻塞 spawn_oopif_arm_loop 的插入）。manager 克隆不复制后台循环，但经共享 Arc
        // 读同一份 context 真相。D1：用 active tab 的 oopif_managers（克隆出的 Arc）。
        let oopif_managers: Vec<(String, InjectionManager)> = {
            let guard = handles
                .oopif_managers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard
                .iter()
                .map(|(sid, entry)| (sid.clone(), entry.manager.clone()))
                .collect()
        };
        for (oopif_session, manager) in oopif_managers {
            // TODO(verify-oopif): 跨源 OOPIF 须 http fixture / 真页验；此处架构接线。
            let Ok(fids) = manager.frame_ids().await else {
                continue;
            };
            for fid in &fids {
                match self
                    .snapshot_one_frame(
                        &manager,
                        fid,
                        next_seq,
                        opts,
                        retained_ref_count,
                        retained_snapshot_json_bytes,
                        retained_frame_bytes,
                    )
                    .await
                {
                    Ok(Some((snap, frame_ref_count, frame_json_bytes, frame_heap_bytes))) => {
                        retained_ref_count = retained_ref_count.saturating_add(frame_ref_count);
                        retained_snapshot_json_bytes = retained_snapshot_json_bytes
                            .saturating_add(frame_json_bytes);
                        retained_frame_bytes =
                            retained_frame_bytes.saturating_add(frame_heap_bytes);
                        snapshot_ref_owners.push((manager.clone(), fid.clone()));
                        if frame_hit_depth_limit(&snap, opts.max_depth) {
                            truncated = true;
                        }
                        // D5：OOPIF 子帧 password ref（其自有 utility world）。失败 → fail-closed。
                        if collect_password_refs(&manager, fid, &mut password_refs).await {
                            any_password_query_failed = true;
                        }
                        frames.push(ObservedFrame {
                            seq: next_seq,
                            frame_id: fid.clone(),
                            session_id: oopif_session.clone(),
                            snapshot: snap,
                        });
                        next_seq += 1;
                    }
                    Ok(None) => {}
                    Err(error @ InjectError::RefCapacityExceeded { .. }) => {
                        Self::invalidate_observe_ref_state(
                            &handles.ref_table,
                            &snapshot_ref_owners,
                        )
                        .await;
                        return Err(map_inject_err(error));
                    }
                    Err(error @ InjectError::ObservationCapacityExceeded { .. }) => {
                        Self::invalidate_observe_ref_state(
                            &handles.ref_table,
                            &snapshot_ref_owners,
                        )
                        .await;
                        return Err(map_inject_err(error));
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "nomi_browser_engine::backend::cdp",
                            frame_id = %fid, error = ?e,
                            "snapshot OOPIF frame failed (skip) (TODO(verify-oopif))"
                        );
                    }
                }
            }
        }

        // 没拍到任何帧（极端：context 始终没就绪）→ 报 context 未就绪而非空白。
        if frames.is_empty() {
            return Err(BrowserError::NavFailed {
                kind: "context".into(),
            });
        }

        // 4) 建 iframe→子帧 路由：child_frame_id → (parent_frame_id, parent_iframe_ref)。
        let parent_of = self.build_iframe_routing(&handles, &frames).await;

        // 5) 自主帧（seq=0 / frame_id == main_frame_id）起递归缝合。
        let main_idx = frames
            .iter()
            .position(|f| f.frame_id == handles.main_frame_id)
            .unwrap_or(0);
        let stitched = match render_frame_recursive_bounded(&frames, main_idx, &parent_of) {
            Ok(stitched) => stitched,
            Err(error) => {
                Self::invalidate_observe_ref_state(
                    &handles.ref_table,
                    &snapshot_ref_owners,
                )
                .await;
                return Err(map_observation_capacity_err(error));
            }
        };

        // 6) D5 password value 置空 → 脱敏 → 不可信包裹（origin = 当前 url）。
        //    正常路径：按 utility world 收集到的 password ref 精确抹掉内联 value（DOM type=password
        //    信号，不误伤普通 textbox）。**fail-closed**：若任一帧 password 探测失败，无法精确知道
        //    哪些是 password，额外对全部可编辑控件值整体 over-redact 兜底（绝不放行明文）。
        //    之后再跑正则/高熵脱敏，最后 <data> 包裹。
        //
        // SD-4：同一次 nav-history 查询同时取 url + POST 标志（避免额外 CDP round-trip）。
        let (url, current_page_is_post) = self.url_and_post_flag(&page_session).await;
        if let Some(url) = &url
            && let Err(error) = ensure_observation_bytes(url.len())
        {
            Self::invalidate_observe_ref_state(&handles.ref_table, &snapshot_ref_owners).await;
            return Err(map_observation_capacity_err(error));
        }
        let blanked = redact::blank_secret_values(&stitched, &password_refs);
        if let Err(error) = ensure_observation_bytes(blanked.len()) {
            Self::invalidate_observe_ref_state(&handles.ref_table, &snapshot_ref_owners).await;
            return Err(map_observation_capacity_err(error));
        }
        let blanked = if any_password_query_failed {
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                "password 探测失败，对可编辑控件值整体 over-redact 以防泄露 (fail-closed)"
            );
            redact::blank_all_editable_values(&blanked)
        } else {
            blanked
        };
        if let Err(error) = ensure_observation_bytes(blanked.len()) {
            Self::invalidate_observe_ref_state(&handles.ref_table, &snapshot_ref_owners).await;
            return Err(map_observation_capacity_err(error));
        }
        let redacted = redact::redact_yaml(&blanked);
        if let Err(error) = ensure_observation_bytes(redacted.len()) {
            Self::invalidate_observe_ref_state(&handles.ref_table, &snapshot_ref_owners).await;
            return Err(map_observation_capacity_err(error));
        }
        let yaml = redact::wrap_untrusted(&redacted, url.as_deref());
        if let Err(error) = ensure_observation_bytes(yaml.len()) {
            Self::invalidate_observe_ref_state(&handles.ref_table, &snapshot_ref_owners).await;
            return Err(map_observation_capacity_err(error));
        }

        // 7) 代际翻新 + entries/ref 表。注意：ref 表与 entries 用**脱敏前**的 stitched 解析
        //    （脱敏只动 secret 文本，不动 role/ref；但用 stitched 保证 ref 行完整不被 <data> 包裹干扰）。
        //    D1：锁 active tab 的 ref_table（克隆出的 Arc，per-tab 隔离）。
        // Build the candidate table off to the side.  It becomes authoritative
        // only after the complete Observation (yaml/entries/url/boxes) passes
        // its retained-byte validation.
        let parsed_generation = {
            let guard = handles.ref_table.lock().await;
            let mut table = RefTable::new_generation(guard.as_ref());
            drop(guard);
            let generation_id = table.generation();
            match Self::parse_refs_into_table(&frames, &stitched, &mut table) {
                Ok(entries) => Ok((generation_id, entries, table)),
                Err(error) => Err(error),
            }
        };
        let (generation, entries, table) = match parsed_generation {
            Ok(generation) => generation,
            Err(error) => {
                Self::invalidate_observe_ref_state(
                    &handles.ref_table,
                    &snapshot_ref_owners,
                )
                .await;
                return Err(BrowserError::Blocked {
                    reason: format!(
                        "observe ref parsing exceeded the task generation limit (limit={}). \
                         The partial generation was discarded; simplify the page or reduce \
                         observe depth, then run a fresh observe",
                        error.limit
                    ),
                });
            }
        };

        let observation = Observation {
            generation,
            yaml,
            entries,
            url,
            truncated,
            current_page_is_post,
            boxes: ref_boxes,
        };
        if let Err(error) = observation.validate_retained_bytes() {
            Self::invalidate_observe_ref_state(&handles.ref_table, &snapshot_ref_owners).await;
            return Err(map_observation_capacity_err(error));
        }
        *handles.ref_table.lock().await = Some(table);
        Ok(observation)
    }

    /// Invalidate both halves of an observe generation that cannot be
    /// published safely.  The Rust generation is advanced to an empty table
    /// first, so old refs become stale immediately; every already-accepted
    /// frame map is then cleared best-effort.  The frame whose bounded snapshot
    /// detected overflow has already cleared itself inside that same JS call.
    async fn invalidate_observe_ref_state(
        ref_table: &std::sync::Arc<AsyncMutex<Option<RefTable>>>,
        owners: &[(InjectionManager, String)],
    ) {
        {
            let mut guard = ref_table.lock().await;
            *guard = Some(RefTable::new_generation(guard.as_ref()));
        }
        for (manager, frame_id) in owners {
            if let Err(error) = manager.clear_snapshot_refs(frame_id).await {
                tracing::warn!(
                    target: "nomi_browser_engine::backend::cdp",
                    frame_id = %frame_id,
                    %error,
                    "failed to clear a discarded observe ref map"
                );
            }
        }
    }

    /// 短轮询等主帧的 utility-world context 就绪（fresh navigate 后 world 物化有延迟）。
    /// 超时 → `NavFailed{kind:"context"}`（语义：这次没拿到可用上下文，调用方可重试）。
    /// **D1**：用传入 active tab 句柄的注入管线 + 主帧 id（observe_impl 已 clone 出，不再读字段）。
    async fn wait_main_context_ready(&self, handles: &TabHandles) -> Result<(), BrowserError> {
        let deadline = tokio::time::Instant::now() + OBSERVE_CONTEXT_READY_TIMEOUT;
        loop {
            if handles
                .injection
                .context_id_for(&handles.main_frame_id)
                .is_ok()
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BrowserError::NavFailed {
                    kind: "context".into(),
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// 拍单帧：取 body objectId → `incrementalAriaSnapshot(body, {mode:ai, refPrefix:f<seq>, depth, track})`
    /// → 在注入调用内校验 task-wide retained-ref budget → 反序列化成
    /// `(FrameSnapshot, retained_ref_count)`。context 未就绪 / body 取不到 → `Ok(None)`
    /// （best-effort 跳过）；容量超限会先清掉本帧 map 再显式返回错误。
    async fn snapshot_one_frame(
        &self,
        manager: &InjectionManager,
        frame_id: &str,
        seq: u32,
        opts: &ObserveOpts,
        already_retained: usize,
        already_retained_json_bytes: usize,
        already_retained_frame_bytes: usize,
    ) -> Result<Option<(FrameSnapshot, usize, usize, usize)>, InjectError> {
        // context 未就绪 / body null → 视为该帧暂不可观测，跳过（不报错）。
        let body_obj_id = match manager.body_object_id(frame_id).await {
            Ok(id) => id,
            Err(InjectError::ContextNotReady { .. }) => return Ok(None),
            // body 为 null（空文档）→ Protocol；当作不可观测帧跳过。
            Err(InjectError::Protocol(_)) => return Ok(None),
            Err(other) => return Err(other),
        };

        let prefix = frame_prefix(seq);
        let node_arg = CallArgument {
            object_id: Some(RemoteObjectId::new(body_obj_id)),
            ..Default::default()
        };
        let opts_arg = CallArgument {
            value: Some(serde_json::json!({
                "mode": "ai",
                "refPrefix": prefix,
                "depth": opts.max_depth,
                "track": if opts.diff { "observe" } else { "" },
            })),
            ..Default::default()
        };
        let (value, retained, reported_serialized_bytes) = manager
            .incremental_aria_snapshot_bounded(
                frame_id,
                node_arg,
                opts_arg,
                already_retained,
                already_retained_json_bytes,
            )
            .await?;
        let serialized_bytes = match serialized_json_bytes_bounded(&value) {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = manager.clear_snapshot_refs(frame_id).await;
                return Err(InjectError::ObservationCapacityExceeded {
                    limit: error.limit,
                    current: already_retained_json_bytes.saturating_add(error.attempted),
                    frame_bytes: error.attempted,
                });
            }
        };
        let attempted_json_bytes = already_retained_json_bytes.saturating_add(serialized_bytes);
        if attempted_json_bytes > MAX_OBSERVATION_RETAINED_BYTES
            || reported_serialized_bytes != serialized_bytes
        {
            let _ = manager.clear_snapshot_refs(frame_id).await;
            return Err(InjectError::ObservationCapacityExceeded {
                limit: MAX_OBSERVATION_RETAINED_BYTES,
                current: attempted_json_bytes.max(
                    already_retained_json_bytes.saturating_add(reported_serialized_bytes),
                ),
                frame_bytes: serialized_bytes.max(reported_serialized_bytes),
            });
        }
        let snap: FrameSnapshot = match serde_json::from_value(value) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                let _ = manager.clear_snapshot_refs(frame_id).await;
                return Err(InjectError::Protocol(
                    "bounded FrameSnapshot payload had an invalid shape".into(),
                ));
            }
        };
        let frame_heap_bytes = match snap.retained_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = manager.clear_snapshot_refs(frame_id).await;
                return Err(InjectError::ObservationCapacityExceeded {
                    limit: error.limit,
                    current: already_retained_frame_bytes.saturating_add(error.attempted),
                    frame_bytes: error.attempted,
                });
            }
        };
        let attempted = already_retained_frame_bytes.saturating_add(frame_heap_bytes);
        if attempted > MAX_OBSERVATION_RETAINED_BYTES {
            let _ = manager.clear_snapshot_refs(frame_id).await;
            return Err(InjectError::ObservationCapacityExceeded {
                limit: MAX_OBSERVATION_RETAINED_BYTES,
                current: attempted,
                frame_bytes: frame_heap_bytes,
            });
        }
        Ok(Some((snap, retained, serialized_bytes, frame_heap_bytes)))
    }

    /// 建 iframe→子帧 路由表：`child_frame_id → (parent_frame_id, parent_iframe_ref)`。
    ///
    /// 对每个**已观测**子帧，`DOM.getFrameOwner(childFrameId)` → owner iframe 的 backendNodeId →
    /// `DOM.resolveNode(backendNodeId, executionContextId=parent_ctx)` → 父 utility world 的 objectId →
    /// `callFunctionOn(读 this._ariaRef.ref)` 拿父帧给该 iframe 元素分配的 ref。best-effort：
    /// 任一步失败该子帧不缝合（仍作为独立帧出现在输出，但不内联）。
    /// **D1**：用传入 active tab 句柄（主帧 id + 注入管线 context 反查）。
    async fn build_iframe_routing(
        &self,
        handles: &TabHandles,
        frames: &[ObservedFrame],
    ) -> HashMap<String, (String, String)> {
        let mut parent_of: HashMap<String, (String, String)> = HashMap::new();
        // frame_id → seq → prefix，用于在父帧的 iframe_refs 里确认归属（也可仅用 ref 字符串）。
        let frame_by_id: HashMap<&str, &ObservedFrame> =
            frames.iter().map(|f| (f.frame_id.as_str(), f)).collect();

        for child in frames {
            // 主帧无 owner iframe，跳过。
            if child.frame_id == handles.main_frame_id {
                continue;
            }
            if let Some((parent_fid, iref)) =
                self.resolve_owner_iframe_ref(handles, child, &frame_by_id).await
            {
                parent_of.insert(child.frame_id.clone(), (parent_fid, iref));
            }
        }
        parent_of
    }

    /// 解析单个子帧的 owner iframe ref（见 [`Self::build_iframe_routing`]）。任一步失败 → None。
    /// **D1**：用传入 active tab 句柄的注入管线做父帧 utility context 反查。
    ///
    /// **owner iframe 元素属于父帧的 target，不属子帧自己的 target**：
    /// - 同进程 iframe：父帧与子帧共用 page session，owner 在 page session。
    /// - 跨进程 OOPIF：owner `<iframe>` 占位元素在**父 target 的渲染进程**里，子帧另起独立
    ///   session；在 OOPIF **自身 session** 上 `getFrameOwner(自身根帧)` 必报 `-32000
    ///   "Frame ... does not belong to the target"`（实测，见 PLATFORM-VERIFICATION「OOPIF 缝合」）。
    ///
    /// 故对每个**候选父帧**，在**该候选父帧的 session** 上发 `getFrameOwner(childFrameId)`：
    /// 只有真父帧的 target 持有该 child 的 owner 元素 → 成功返 backendNodeId；非父帧报错跳过。
    /// 再 `resolveNode(backendNodeId, ctx=父 utility)` + 读 `_ariaRef.ref`，并以「ref ∈ 父帧
    /// iframe_refs」二次确认归属（防同进程下他帧 backendNodeId 误配）。
    ///
    /// 局限（已知、graceful degrade）：**嵌套 OOPIF**（OOPIF 内再嵌跨站 OOPIF）的父帧是中间
    /// OOPIF，其 utility context 不在 `handles.injection`（page session 管线）里 → `context_id_for`
    /// 取不到 → 该子帧不内联（仍作独立帧出现）。一级 OOPIF（父=主帧/page session）完整缝合。
    async fn resolve_owner_iframe_ref(
        &self,
        handles: &TabHandles,
        child: &ObservedFrame,
        frame_by_id: &HashMap<&str, &ObservedFrame>,
    ) -> Option<(String, String)> {
        for (pfid, pframe) in frame_by_id.iter() {
            if *pfid == child.frame_id {
                continue;
            }
            // a) getFrameOwner 在**候选父帧的 session** 上发——owner iframe 元素属父 target。childFrameId
            //    不属该 target（非真父 / OOPIF 自身 session）时报 -32000 → 跳过。同进程下所有帧共用
            //    page session,故任一候选都返同一 owner backendNodeId,靠下方 _ariaRef.ref 归属确认。
            let Ok(owner) = self
                .conn
                .send::<GetFrameOwnerParams>(
                    &pframe.session_id,
                    &GetFrameOwnerParams::new(child.frame_id.clone()),
                )
                .await
            else {
                continue;
            };
            let Some(backend_node_id) = owner.get("backendNodeId").and_then(|v| v.as_i64()) else {
                continue;
            };
            // b) 父帧 utility context（page session 管线反查；嵌套 OOPIF 父帧取不到 → 跳过,见 doc 局限）。
            let Ok(parent_ctx) = handles.injection.context_id_for(pframe.frame_id.as_str()) else {
                continue;
            };
            // c) resolveNode 到父帧 utility world。
            let resolve = ResolveNodeParams {
                node_id: None,
                backend_node_id: Some(
                    chromiumoxide::cdp::browser_protocol::dom::BackendNodeId::new(backend_node_id),
                ),
                object_group: None,
                execution_context_id: Some(ExecutionContextId::new(parent_ctx)),
            };
            let Ok(resolved) = self
                .conn
                .send::<ResolveNodeParams>(&pframe.session_id, &resolve)
                .await
            else {
                continue;
            };
            let Some(obj_id) = resolved
                .get("object")
                .and_then(|o| o.get("objectId"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            // d) 读该 iframe 元素被父帧 incrementalAriaSnapshot 分配的 _ariaRef.ref。
            let mut call = CallFunctionOnParams::new(
                "function() { return this && this._ariaRef ? this._ariaRef.ref : null; }"
                    .to_string(),
            );
            call.object_id = Some(RemoteObjectId::new(obj_id.to_string()));
            call.return_by_value = Some(true);
            let Ok(call_res) = self
                .conn
                .send::<CallFunctionOnParams>(&pframe.session_id, &call)
                .await
            else {
                continue;
            };
            if let Some(reff) = call_res
                .get("result")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
            {
                // 确认该 ref 确在父帧的 iframe_refs 里（防误配）。
                if pframe.snapshot.iframe_refs.iter().any(|r| r == reff) {
                    return Some((pframe.frame_id.clone(), reff.to_string()));
                }
            }
        }
        None
    }

    /// 解析缝合后 YAML 里每行的 `[ref=...]`，填 [`RefTable`] + 产出 [`ElementEntry`] 列表。
    /// ref 的归属帧（session_id/frame_id/frame_seq）按 ref 的 `f<seq>` 前缀回查 frames。
    fn parse_refs_into_table(
        frames: &[ObservedFrame],
        stitched: &str,
        table: &mut RefTable,
    ) -> Result<Vec<ElementEntry>, RefTableCapacityError> {
        // seq → (session_id, frame_id)。
        let by_seq: HashMap<u32, (&str, &str)> = frames
            .iter()
            .map(|f| (f.seq, (f.session_id.as_str(), f.frame_id.as_str())))
            .collect();
        let mut entries = Vec::new();
        let mut records = Vec::new();
        let mut seen = HashSet::new();
        for line in stitched.lines() {
            let Some(reff) = parse_ref_token(line) else {
                continue;
            };
            // ref = f<seq>e<n>：抽 seq 定位帧。
            let Some(seq) = parse_seq_from_ref(&reff) else {
                continue;
            };
            let Some((session_id, frame_id)) = by_seq.get(&seq).copied() else {
                // Never publish a syntactically plausible ref whose frame was
                // not part of this authoritative observe.  It cannot resolve
                // and may originate from untrusted text inside the YAML.
                continue;
            };
            // Snapshot refs are expected to be unique.  Dedup defensively so
            // repeated YAML lines cannot inflate Observation.entries, and
            // stop before allocating any record beyond the hard generation
            // bound.  The RefTable itself is untouched until the full batch is
            // known to fit, so failure never publishes a partial generation.
            if !seen.insert(reff.clone()) {
                continue;
            }
            if seen.len() > MAX_REFS_PER_GENERATION {
                return Err(RefTableCapacityError {
                    limit: MAX_REFS_PER_GENERATION,
                });
            }
            let (role, name) = parse_role_name(line);
            records.push((
                reff.clone(),
                RefRecord {
                    session_id: session_id.to_string(),
                    frame_id: frame_id.to_string(),
                    full_ref: reff.clone(),
                    role: role.clone(),
                    name: name.clone(),
                },
            ));
            entries.push(ElementEntry {
                r#ref: reff,
                role,
                name,
                frame_seq: seq,
            });
        }
        table.try_insert_batch(records)?;
        Ok(entries)
    }

    // ── act 反查（P2 命脉，actionability.rs）需要的内部访问器 ──────────────────────
    // resolve_ref_to_object / release_act_group 据 RefRecord.session_id 选注入管线、并报
    // NodeStale 时带当前代际。这些访问器把 CdpBackend 的私有字段以受控只读面暴露给同 crate 的
    // actionability 模块（避免把字段全 pub）。
    //
    // **D1 结构改造**：per-tab 字段下放进 TabRecord 后，这些访问器**不再返引用**（字段在
    // `tabs` 锁后的 HashMap 值里），改为**异步返克隆出的 owned 值**——经 active_tab_handles 短暂锁
    // tabs/active_target 克隆出句柄后立即释放（不跨 await 持 tabs 锁）。active tab 缺失 → 返 Err
    // （绝不 panic）。conn / next_act_seq 非 per-tab，保留同步引用/原子语义。

    /// active tab 的 page sessionId（actionability 据此判 RefRecord 属主帧还是 OOPIF 子帧）。
    /// `pub`：act 反查测试 / facade 构造 RefRecord 路由时需要。**D1：async 返 owned**（active tab
    /// 缺失 → Err）。
    pub async fn page_session_id(&self) -> Result<String, BrowserError> {
        Ok(self.active_tab_handles().await?.session_id)
    }

    /// active tab 的主帧 frameId（== page targetId，CDP 约定）。`pub`：同上，反查路由 / 测试构造记录用。
    /// **D1：async 返 owned**。
    pub async fn main_frame_id(&self) -> Result<String, BrowserError> {
        Ok(self.active_tab_handles().await?.main_frame_id)
    }

    /// active tab 的注入管线（同进程帧的 ref 反查走它；`Clone` 共享 Arc 缓存）。**D1：async 返克隆**。
    pub(crate) async fn injection_manager(&self) -> Result<InjectionManager, BrowserError> {
        Ok(self.active_tab_handles().await?.injection)
    }

    /// 底层 CDP 连接（B5 输入合成 / 后续 act 经它发裸 `Input.*` / `DOM.getContentQuads` 命令）。
    /// `pub(crate)`：input.rs 的输入合成方法据此把 active session 喂给收 `&Connection` 的自由
    /// 函数；不对外暴露（外部经 act facade 用，非裸连接）。**非 per-tab，保留同步引用**。
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 取 active tab 里某 OOPIF 子 session 的注入管线（克隆出句柄，锁外用）。未 arm/已 detach /
    /// active tab 缺失 → None。**D1：经 active tab 的 oopif_managers 解引用**。
    pub(crate) async fn oopif_manager_for(&self, session_id: &str) -> Option<InjectionManager> {
        let handles = self.active_tab_handles().await.ok()?;
        handles
            .oopif_managers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .map(|entry| entry.manager.clone())
    }

    /// active tab 当前（最近一次 observe 的）ref 表代际，供 actionability 报
    /// [`BrowserError::NodeStale`] 时带上。还没 observe 过（表为空）/ active tab 缺失 → 0（哨兵代际，
    /// 语义「任何 ref 都 stale」）。**D1：经 active tab 的 ref_table 解引用**。
    pub(crate) async fn current_generation(&self) -> u64 {
        let Ok(handles) = self.active_tab_handles().await else {
            return 0;
        };
        handles
            .ref_table
            .lock()
            .await
            .as_ref()
            .map(|t| t.generation().0)
            .unwrap_or(0)
    }

    /// **取下一个 act objectGroup 序号**（C1）：`fetch_add(1, Relaxed)` 返回当前值并自增。每次
    /// `act` 调一次，拼成本动作的 objectGroup `act-<seq>`，保证连续/并发动作的句柄组互不串味。
    /// **非 per-tab（全局），保留原子语义**。
    pub(crate) fn next_act_seq(&self) -> u64 {
        self.act_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// active tab 的 ref 表锁的 **Arc 克隆**（actionability 层① 在当前代际表里 `resolve(llm_ref)` 取
    /// [`crate::aria_ref::RefRecord`]）。返 Arc（非引用）让 actionability 在同一临界区内既读 generation
    /// 又 resolve（避免两次锁的 TOCTOU：observe 可能在两次锁之间翻新代际），且**不**让调用方跨 await
    /// 持 `tabs` 锁——clone 出 Arc 后独立锁它。active tab 缺失 → Err。**D1：async 返 per-tab ref_table 的 Arc**。
    pub(crate) async fn ref_table_lock(
        &self,
    ) -> Result<std::sync::Arc<AsyncMutex<Option<RefTable>>>, BrowserError> {
        Ok(self.active_tab_handles().await?.ref_table)
    }

    /// **[仅测试支持]** 在主 page 的**默认（页面）world** `Runtime.evaluate` 一段副作用脚本，用于
    /// 集成测试在 observe 之后**改 DOM 活元素状态**（如把某元素 `display:none` 制造「ref 已分配但
    /// 现已不可见」场景，验证 actionability 五检查的 `visible` 判定）。`#[doc(hidden)]`：非产品 API，
    /// 仅 `tests/integration_act.rs` 等用；走默认 world（而非 utility world）以便直接操作页面 DOM。
    /// 返回 `result.result` RemoteObject（by-value）。失败/JS 异常返 `Err`（绝不 panic）。
    /// **D1：经 active tab 的 session 发**。
    #[doc(hidden)]
    pub async fn __eval_page_world_for_test(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, BrowserError> {
        let session = self.active_tab_handles().await?.session_id;
        let mut params = EvaluateParams::new(expression.to_string());
        params.return_by_value = Some(true);
        params.await_promise = Some(false);
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(BrowserError::Other(format!("test eval threw: {ex}")));
        }
        Ok(result.get("result").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// **[仅测试支持]** 同 [`Self::__eval_page_world_for_test`]，但 `awaitPromise = true`——
    /// 用于 async JS 表达式（返回 Promise 的 IIFE）。等 Promise resolve 后返回 by-value 结果。
    #[doc(hidden)]
    pub async fn __eval_page_world_await_for_test(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, BrowserError> {
        let session = self.active_tab_handles().await?.session_id;
        let mut params = EvaluateParams::new(expression.to_string());
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(BrowserError::Other(format!("test eval threw: {ex}")));
        }
        Ok(result.get("result").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// **[仅测试支持]** 关闭 active tab 的 page target（`Target.closeTarget{targetId}`，发到根 browser
    /// session），模拟「用户关掉标签页」——CDP 随之发 `Target.detachedFromTarget`（sessionId == 本 page
    /// session）。B6 集成测试用它触发 detach 事件源，验证 [`Self::arm_act_abort`] →
    /// `progress.abort(PageClosed)` → 进行中的 [`crate::actions::run_act_with_retry`] 立即以
    /// `TargetClosed` 返回（远早于 deadline）。`#[doc(hidden)]`：非产品 API，仅 `tests/integration_act.rs`
    /// 用。失败返 `Err`（绝不 panic）。**D1：关 active tab 的 target**。
    #[doc(hidden)]
    pub async fn __close_page_target_for_test(&self) -> Result<(), BrowserError> {
        use chromiumoxide::cdp::browser_protocol::target::CloseTargetParams;
        let target_id = self.active_tab_handles().await?.target_id;
        let params = CloseTargetParams::new(target_id);
        self.conn
            .send::<CloseTargetParams>(ROOT_SESSION, &params)
            .await
            .map_err(map_transport_err)?;
        Ok(())
    }

    /// **[仅测试支持]** 读回引擎构造期注入的出口防火墙配置（P3-G1 注入链验证）。`firewall_loop`
    /// 在后台任务里消费该配置、无法从外部直接观测；本 accessor 读回**与 loop 同值**的快照
    /// （[`Self::firewall_config`]），使 `#[ignore]` 集成测试能断言「自定义 FirewallConfig 真的注入到
    /// 了引擎」而非被硬编码 `default()` 吞掉。`#[doc(hidden)]`：非产品 API，仅集成测试用。
    #[doc(hidden)]
    pub fn firewall_config_for_test(&self) -> crate::firewall::FirewallConfig {
        // P3-D1：FirewallConfig 不再 Copy（含 Vec 域名策略字段）→ clone 返回。
        self.firewall_config.clone()
    }

    /// **[仅测试支持]** 当前 browser 里 `type=="page"` 的 target 总数（经 `Target.getTargets`）。
    /// 用于断言「启动后**恰好一个**受控 page」——验证 `--no-startup-window` 消除了命令行冗余
    /// about:blank 启动标签（旧行为 = 命令行 about:blank + createTarget 受控页 = **2** 个 page；
    /// 新行为 = 仅 createTarget 受控页 = **1** 个 page）。`#[doc(hidden)]`：非产品 API。
    #[doc(hidden)]
    pub async fn page_target_count_for_test(&self) -> Result<usize, BrowserError> {
        use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
        let raw = self
            .conn
            .send::<GetTargetsParams>(ROOT_SESSION, &GetTargetsParams { filter: None })
            .await
            .map_err(map_transport_err)?;
        let count = raw
            .get("targetInfos")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|ti| ti.get("type").and_then(|v| v.as_str()) == Some("page"))
                    .count()
            })
            .unwrap_or(0);
        Ok(count)
    }

    /// **OOPIF 验证 seam**：active tab 当前已 arm 的跨进程 OOPIF 子 session 数（`oopif_managers` 长度）。
    /// 真跨源 iframe（Chrome site-isolation 把它另起 `type=="iframe"` 子 session）才 >0;同进程 iframe
    /// （同源 / `srcdoc`）不另起子 session,恒 0。供 `integration_oopif` 断言「跨源 OOPIF 子 session 真被
    /// arm」（`spawn_oopif_arm_loop` 接线在真 http 多源页才走得到,离线 file:// 触发不到）。
    pub async fn oopif_session_count_for_test(&self) -> usize {
        match self.active_tab_handles().await {
            Ok(handles) => handles
                .oopif_managers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            Err(_) => 0,
        }
    }

    /// **W4b：捕获默认 browser context 的全部 cookie → storage_state**（DESIGN §17）。
    ///
    /// 用 **`Storage.getCookies`**（不传 `browserContextId`）取**默认 browser context** 的所有
    /// cookie（**全字段保真**：CHIPS partitionKey + sameSite + domain/path/expires/httpOnly/secure +
    /// priority/sourceScheme/sourcePort），序列化进 [`crate::storage_state::StorageState`]。
    ///
    /// 失败 → [`BrowserError`]（绝不 panic）。
    pub async fn capture_cookies(&self) -> Result<crate::storage_state::StorageState, BrowserError> {
        let params = StorageGetCookiesParams::default();
        let raw = self
            .conn
            .send::<StorageGetCookiesParams>(ROOT_SESSION, &params)
            .await
            .map_err(map_transport_err)?;
        // 反序列化回包的 `cookies` 数组（Vec<network::Cookie>）。缺键 / 空 → 空（无 cookie 是合法态）。
        let cookies: Vec<chromiumoxide::cdp::browser_protocol::network::Cookie> = raw
            .get("cookies")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                BrowserError::Other(format!("Storage.getCookies response parse failed: {e}"))
            })?
            .unwrap_or_default();
        let state = crate::storage_state::StorageState::from_cdp_cookies(cookies);
        state.validate_bounds().map_err(|error| BrowserError::Blocked {
            reason: format!("cookie identity exceeds its per-task hard boundary: {error}"),
        })?;
        Ok(state)
    }

    /// **持久登录：组合捕获当前登录态**（[`Self::capture_cookies`] 全域 cookie +
    /// [`Self::capture_local_storage`] 当前 origin 的 localStorage + [`Self::capture_index_db`]
    /// best-effort）。cookie 是登录态主载体（全域采全）;localStorage/IndexedDB origin-bound,
    /// 只采当前 active tab 那个 origin。localStorage 采不到（about:blank / 无 location）→ 只带 cookie。
    /// IndexedDB 采集失败 → best-effort 忽略（`index_db=None`），绝不因此让整体 capture 失败。
    pub async fn capture_storage_state(
        &self,
    ) -> Result<crate::storage_state::StorageState, BrowserError> {
        let mut state = self.capture_cookies().await?;
        if let Some(origin_storage) = self.capture_local_storage().await? {
            // IndexedDB is deliberately omitted. A page-side `getAll()` can materialize an
            // arbitrarily large object before CDP or Rust can enforce any byte limit.
            state.local_storage.push(origin_storage);
        }
        state.validate_bounds().map_err(|error| BrowserError::Blocked {
            reason: format!("captured identity exceeds its per-task hard boundary: {error}"),
        })?;
        Ok(state)
    }

    /// **W4b：把 storage_state 的 cookie 灌进默认 browser context（恢复登录态）**（DESIGN §17）。
    ///
    /// 用 **`Storage.setCookies`**（不传 `browserContextId`）把 [`crate::storage_state::StorageState`]
    /// 的 cookie 转成 `Network.CookieParam`（partitionKey/sameSite **原样灌**）写进**默认 context**——
    /// 恢复跨会话登录态。命令发到**根 browser session**（ROOT_SESSION）。
    ///
    /// **幂等/可重入**：`Storage.setCookies` 按 (name,domain,path,partitionKey) upsert，重复灌同一份
    /// storage_state 不产生重复 cookie。空 cookie 数组 → no-op（无登录态可灌）。
    ///
    /// 失败 → [`BrowserError`]（绝不 panic）。
    pub async fn restore_cookies(
        &self,
        state: &crate::storage_state::StorageState,
    ) -> Result<(), BrowserError> {
        validate_storage_state_for_restore(state)?;
        let cookies = state.to_cookie_params();
        // 空 → no-op（setCookies 灌空数组无意义，且 cookies 字段 skip_serializing_if Vec::is_empty）。
        if cookies.is_empty() {
            return Ok(());
        }
        let params = StorageSetCookiesParams::new(cookies);
        self.conn
            .send::<StorageSetCookiesParams>(ROOT_SESSION, &params)
            .await
            .map_err(map_transport_err)?;
        Ok(())
    }

    /// **W4c：捕获**当前页面 origin 的 **localStorage** → 一个 [`crate::storage_state::OriginStorage`]
    /// （origin-bound，DESIGN §17）。
    ///
    /// **origin-bound 现实**：localStorage 按 origin 分区（同源策略）——一个文档只能读到**自己 origin**
    /// 的 localStorage，无法跨 origin 枚举。故捕获只能取**当前 active tab 页面已加载的那个 origin**
    /// 的 localStorage（caller 先 navigate 到目标 origin，再 capture）。这与 cookie 的 `Storage.getCookies`
    /// （能按 browserContextId 取全 context cookie）不同——localStorage 没有「按 context 取全 origin」的
    /// CDP 面，必须 per-origin 在页面上下文采。
    ///
    /// 注入脚本 `(()=>{ ... Object.entries(localStorage) ... })()`（默认 page world `Runtime.evaluate`，
    /// by-value）——返回 `{ origin, items:[[k,v],...] }`。`file://` 等 opaque origin（`location.origin`
    /// 形如 `"null"` / `"file://"`）也照样采（其 localStorage 仍是该文档的）。localStorage 访问被禁
    /// （如某些 sandbox / disabled storage）→ try/catch 兜底返回空 items（绝不 panic / 不抛）。
    ///
    /// 返回 `Ok(None)` 当当前页面**无可采 origin**（无 location）；否则 `Ok(Some(OriginStorage))`
    /// （items 可能为空 = 该 origin 无 localStorage 项，仍是合法快照）。IndexedDB best-effort（TODO）不采
    /// （`index_db=None`）。**经 active tab 的 page session 发**（D1）。
    pub async fn capture_local_storage(
        &self,
    ) -> Result<Option<crate::storage_state::OriginStorage>, BrowserError> {
        // 采当前页面 origin + localStorage 全键值（默认 page world；try/catch 兜底 storage 不可用）。
        // 返回 by-value `{origin, items:[[k,v],...]}`；无 location → origin 为空串。
        let script = format!(
            "(() => {{ try {{ \
                const origin = (location && location.origin) ? location.origin : ''; \
                const items = []; let retainedUtf16Bytes = 0; \
                for (let i = 0; i < localStorage.length; i++) {{ \
                    if (items.length >= {max_items}) return {{ origin, items: [], overflow: true }}; \
                    const k = localStorage.key(i); const v = localStorage.getItem(k); \
                    const itemBytes = ((k || '').length + (v || '').length) * 2; \
                    if (itemBytes > {max_bytes} - retainedUtf16Bytes) \
                        return {{ origin, items: [], overflow: true }}; \
                    retainedUtf16Bytes += itemBytes; items.push([k, v]); \
                }} \
                return {{ origin, items, overflow: false }}; \
            }} catch (e) {{ return {{ origin: (location && location.origin) || '', items: [], overflow: false }}; }} }})()",
            max_items = crate::storage_state::MAX_STORAGE_STATE_LOCAL_ITEMS_PER_ORIGIN,
            max_bytes = crate::storage_state::MAX_CAPTURED_LOCAL_STORAGE_UTF16_BYTES,
        );
        let session = self.active_tab_handles().await?.session_id;
        let mut params = EvaluateParams::new(script);
        params.return_by_value = Some(true);
        params.await_promise = Some(false);
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(BrowserError::Other(format!(
                "capture_local_storage eval threw: {ex}"
            )));
        }
        let value = result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let origin = value
            .get("origin")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // 无 origin（about:blank / 无 location）→ 无可采 origin。
        if origin.is_empty() {
            return Ok(None);
        }
        if value
            .get("overflow")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            tracing::warn!(
                target: "nomi_browser_engine::storage_state",
                %origin,
                "localStorage capture exceeded its per-task hard boundary; omitting this origin"
            );
            return Ok(None);
        }
        let mut items = Vec::new();
        if let Some(arr) = value.get("items").and_then(|v| v.as_array()) {
            for pair in arr {
                if let Some(p) = pair.as_array()
                    && let (Some(k), Some(v)) =
                        (p.first().and_then(|x| x.as_str()), p.get(1).and_then(|x| x.as_str()))
                {
                    items.push((k.to_string(), v.to_string()));
                }
            }
        }
        let origin_storage = crate::storage_state::OriginStorage::new_local_storage(origin, items);
        let probe = crate::storage_state::StorageState {
            cookies: Vec::new(),
            local_storage: vec![origin_storage.clone()],
        };
        probe.validate_bounds().map_err(|error| BrowserError::Blocked {
            reason: format!("captured localStorage exceeds its per-task hard boundary: {error}"),
        })?;
        Ok(Some(origin_storage))
    }

    /// **IndexedDB capture**：采集当前页面 origin 的所有 IndexedDB 数据库 →
    /// [`crate::storage_state::IndexedDbDump`]（origin-bound，DESIGN §17）。
    ///
    /// 注入一段 async JS（utility world 不需要，用默认 page world + `awaitPromise:true`）：
    /// - 枚举 `indexedDB.databases()` 获取所有 DB 名 + 版本。
    /// - 对每个 DB：`indexedDB.open(name, version)` → 遍历 `objectStoreNames` → 对每个 store
    ///   `getAll()` 取全部记录 + 读 `keyPath`/`autoIncrement`。
    /// - 二进制值（ArrayBuffer/typed array）→ base64 哨兵 `{"__b64__":"..."}`。
    /// - 返回结构化 JSON → 映射到 `IndexedDbDump`。
    ///
    /// `indexedDB.databases()` 在部分旧 Chrome (<72) 不可用——此时返回 `Ok(None)`（优雅降级）。
    /// opaque origin（`data:`/`file://`）的 Chrome 行为不稳定——若 `databases()` 失败返回 `Ok(None)`。
    ///
    /// 返回 `Ok(None)` = 当前 origin 无 IndexedDB（或不可枚举）；`Ok(Some(dump))` = 成功采集。
    pub async fn capture_index_db(
        &self,
    ) -> Result<Option<crate::storage_state::IndexedDbDump>, BrowserError> {
        // Full IndexedDB capture is intentionally disabled. The previous implementation used
        // one `getAll()` per object store, so cancellation could detach an unbounded renderer
        // allocation and a retry could stack another one. A future implementation must use a
        // cursor with byte/record/deadline budgets; until then omission is the only hard bound.
        Ok(None)
    }

    /// **W4c：恢复 localStorage（origin-bound 注入）**——把 [`crate::storage_state::StorageState`] 里
    /// **匹配当前页面 origin** 的那个 [`crate::storage_state::OriginStorage`] 的键值，经
    /// `localStorage.setItem` 灌进当前页面（DESIGN §17 / 裁决⑥）。
    ///
    /// **origin-bound 注入的现实 + 本实现的边界（重要）**：
    /// localStorage 写入也受同源策略约束——只能写**当前文档 origin** 的 localStorage。一份 storage_state
    /// 可能含**多个 origin** 的 localStorage（用户在多站登录）；要把每个 origin 的 localStorage 都灌回，
    /// 严格做法是 DESIGN §17 的「**伪空 HTML 导航技巧**」：对每个 origin，用 `Fetch` 拦截器把该 origin 的
    /// 一个 URL 伪造成空 HTML 响应（避免真实网络/登录墙污染）→ `navigate(origin)` → 注入 `setItem`。
    /// 但本引擎的 `Fetch` 域已被**出口防火墙 loop**（[`super::cdp::spawn_fetch_firewall_loop`]）独占，
    /// 在 restore 期临时插一套伪 HTML 拦截会与 firewall loop 竞争同一 `Fetch.requestPaused` 流——这是个
    /// 真实的架构耦合点。
    ///
    /// 故 **W4c 采用「caller-navigated origin-bound 注入」**：本方法只把 **storage_state 中 origin ==
    /// 当前页面 origin** 的那一份 localStorage 注入当前页面（caller 先 `navigate(origin)` 再调本方法）。
    /// 多 origin 恢复 = caller 对每个 origin「navigate → restore」一轮（与捕获对称）。这把 origin-bound
    /// 注入做对（绝不跨 origin 误写），且不与 firewall loop 抢 `Fetch` 流；**伪空 HTML 自动遍历全 origin**
    /// 留作后续增强（见下方 TODO，需引擎层与 firewall loop 协调 `Fetch` 拦截，或改用一次性
    /// `addScriptToEvaluateOnNewDocument` origin-bound storageScript）。
    ///
    /// 行为：取当前页面 `location.origin` → 在 `state.local_storage` 找 origin 相等的项 → 逐键 `setItem`
    /// （存在则覆盖，幂等）。无匹配 origin（state 里没有当前页面 origin 的 localStorage）→ no-op（`Ok`）。
    /// 注入脚本经默认 page world `Runtime.evaluate`；JS 抛异常 → `Err`（绝不 panic）。**经 active tab 的
    /// page session 发**（D1）。
    // TODO(W4-followup): 伪空 HTML 导航自动遍历 state.local_storage 全 origin（DESIGN §17）——需与
    // firewall loop 协调 Fetch 拦截，或用 origin-bound addScriptToEvaluateOnNewDocument storageScript。
    pub async fn restore_local_storage(
        &self,
        state: &crate::storage_state::StorageState,
    ) -> Result<(), BrowserError> {
        validate_storage_state_for_restore(state)?;
        // 无 localStorage 可恢复 → no-op。
        if state.local_storage.is_empty() {
            return Ok(());
        }
        let session = self.active_tab_handles().await?.session_id;
        // 取当前页面 origin（origin-bound：只灌匹配 origin 的那份）。
        let origin = {
            let mut p = EvaluateParams::new(
                "(() => (location && location.origin) ? location.origin : '')()".to_string(),
            );
            p.return_by_value = Some(true);
            let r = self
                .conn
                .send::<EvaluateParams>(&session, &p)
                .await
                .map_err(map_transport_err)?;
            r.get("result")
                .and_then(|x| x.get("value"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        };
        // 在 state 里找 origin == 当前页面 origin 的那份 localStorage（origin-bound）。
        let Some(origin_storage) = state.local_storage.iter().find(|o| o.origin == origin) else {
            // state 里没有当前页面 origin 的 localStorage（caller 还没 navigate 到目标 origin，
            // 或该 origin 无 localStorage 需恢复）→ no-op（不跨 origin 误写）。
            return Ok(());
        };
        if origin_storage.local_storage.is_empty() {
            return Ok(());
        }
        // 把键值数组序列化进脚本（JSON 安全编码 key/value——含引号/反斜杠/换行都不破坏脚本）。
        let pairs: Vec<[&str; 2]> = origin_storage
            .local_storage
            .iter()
            .map(|i| [i.name.as_str(), i.value.as_str()])
            .collect();
        let pairs_json = serde_json::to_string(&pairs)
            .map_err(|e| BrowserError::Other(format!("serialize localStorage pairs: {e}")))?;
        // 注入：逐键 setItem（覆盖即幂等）。try/catch 兜底 storage 不可用（返回失败原因供诊断）。
        let script = format!(
            "(() => {{ try {{ const pairs = {pairs_json}; \
             for (const [k, v] of pairs) {{ localStorage.setItem(k, v); }} \
             return true; \
             }} catch (e) {{ throw new Error('localStorage.setItem failed: ' + e); }} }})()"
        );
        let mut params = EvaluateParams::new(script);
        params.return_by_value = Some(true);
        params.await_promise = Some(false);
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(BrowserError::Other(format!(
                "restore_local_storage eval threw: {ex}"
            )));
        }
        Ok(())
    }

    /// **IndexedDB restore（origin-bound 写回）**——把 [`crate::storage_state::StorageState`] 中
    /// **匹配当前页面 origin** 的 [`crate::storage_state::IndexedDbDump`] 恢复到当前页面的
    /// IndexedDB（origin-bound，DESIGN §17）。
    ///
    /// 行为：取当前页面 `location.origin` → 在 `state.local_storage` 中找 origin 相等且
    /// `index_db = Some(dump)` 的项 → 对 dump 中每个数据库：`indexedDB.open(name, version)` 创建
    /// （onupgradeneeded 中建 objectStore）→ 对每个 store `put` 全部 records（base64 哨兵
    /// `{"__b64__":"..."}` 解码回 ArrayBuffer）。
    ///
    /// **origin-bound**：只恢复 origin == 当前页面 origin 的那份 IDB（caller 先 navigate 到
    /// 目标 origin）。无匹配 origin / 无 index_db → no-op（`Ok`）。
    ///
    /// 注入的 restore JS 经默认 page world `Runtime.evaluate`（`awaitPromise:true`）；JS 抛异常
    /// → `Err`（绝不 panic）。**经 active tab 的 page session 发**。
    pub async fn restore_index_db(
        &self,
        state: &crate::storage_state::StorageState,
    ) -> Result<(), BrowserError> {
        validate_storage_state_for_restore(state)?;
        if state.local_storage.is_empty() {
            return Ok(());
        }
        let session = self.active_tab_handles().await?.session_id;
        // 取当前页面 origin。
        let origin = {
            let mut p = EvaluateParams::new(
                "(() => (location && location.origin) ? location.origin : '')()".to_string(),
            );
            p.return_by_value = Some(true);
            let r = self
                .conn
                .send::<EvaluateParams>(&session, &p)
                .await
                .map_err(map_transport_err)?;
            r.get("result")
                .and_then(|x| x.get("value"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        };
        // 找 origin 匹配且有 index_db 的 OriginStorage。
        let Some(origin_storage) = state.local_storage.iter().find(|o| o.origin == origin) else {
            return Ok(());
        };
        let Some(dump) = &origin_storage.index_db else {
            return Ok(());
        };
        if dump.databases.is_empty() {
            return Ok(());
        }

        // 序列化 dump 为 JSON 注入到 restore JS。
        let dump_json = serde_json::to_string(dump)
            .map_err(|e| BrowserError::Other(format!("serialize IndexedDbDump: {e}")))?;

        // Restore JS: for each DB, open with version (triggering upgradeneeded to create stores),
        // then put all records. Decode __b64__ sentinels back to ArrayBuffer.
        let script = format!(
            r#"(async () => {{
            const dump = {dump_json};

            function decodeValue(val) {{
                if (val === null || val === undefined) return val;
                if (Array.isArray(val)) return val.map(decodeValue);
                if (typeof val === 'object' && val !== null) {{
                    if (val.__b64__ !== undefined) {{
                        const binaryStr = atob(val.__b64__);
                        const bytes = new Uint8Array(binaryStr.length);
                        for (let i = 0; i < binaryStr.length; i++) bytes[i] = binaryStr.charCodeAt(i);
                        return bytes.buffer;
                    }}
                    const out = {{}};
                    for (const [k, v] of Object.entries(val)) out[k] = decodeValue(v);
                    return out;
                }}
                return val;
            }}

            for (const dbInfo of dump.databases) {{
                const db = await new Promise((resolve, reject) => {{
                    const req = indexedDB.open(dbInfo.name, dbInfo.version);
                    req.onupgradeneeded = (e) => {{
                        const db = e.target.result;
                        for (const storeInfo of dbInfo.stores) {{
                            if (!db.objectStoreNames.contains(storeInfo.name)) {{
                                const opts = {{}};
                                if (storeInfo.keyPath) opts.keyPath = storeInfo.keyPath;
                                if (storeInfo.autoIncrement) opts.autoIncrement = true;
                                db.createObjectStore(storeInfo.name, opts);
                            }}
                        }}
                    }};
                    req.onsuccess = () => resolve(req.result);
                    req.onerror = () => reject(req.error);
                }});

                for (const storeInfo of dbInfo.stores) {{
                    if (!db.objectStoreNames.contains(storeInfo.name)) continue;
                    const tx = db.transaction(storeInfo.name, "readwrite");
                    const store = tx.objectStore(storeInfo.name);
                    for (const record of storeInfo.records) {{
                        store.put(decodeValue(record));
                    }}
                    await new Promise((resolve, reject) => {{
                        tx.oncomplete = resolve;
                        tx.onerror = () => reject(tx.error);
                    }});
                }}
                db.close();
            }}
            return "ok";
        }})()"#
        );

        let mut params = EvaluateParams::new(script);
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(BrowserError::Other(format!(
                "restore_index_db eval threw: {ex}"
            )));
        }
        Ok(())
    }

    /// **多 origin localStorage + IndexedDB 自动遍历恢复**——无需 caller 逐 origin 手动 navigate，
    /// 一次调用自动遍历 [`crate::storage_state::StorageState`] 中所有 origin 的 localStorage
    /// （+ IndexedDB）并恢复到对应 origin。
    ///
    /// Each bounded origin is restored with `Page.navigate`, followed by the ordinary
    /// origin-matching `Runtime.evaluate` paths for localStorage and legacy IndexedDB state.
    /// No new-document script is registered: cancellation or an early error therefore cannot
    /// leave browser-side scripts (and their embedded identity payloads) alive on the session.
    /// This remains Fetch-free and does not compete with the engine's firewall event loop.
    pub async fn restore_all_origins(
        &self,
        state: &crate::storage_state::StorageState,
    ) -> Result<(), BrowserError> {
        validate_storage_state_for_restore(state)?;
        if state.local_storage.is_empty() {
            return Ok(());
        }

        let session = self.active_tab_handles().await?.session_id;
        for origin_storage in &state.local_storage {
            if origin_storage.local_storage.is_empty() && origin_storage.index_db.is_none() {
                continue;
            }

            // Navigate first, then inject into the now matching origin. The previous
            // addScriptToEvaluateOnNewDocument approach leaked registered scripts whenever
            // this future was cancelled or a later `?` returned early.
            let nav_params = NavigateParams::new(origin_storage.origin.clone());
            let _ = self
                .conn
                .send::<NavigateParams>(&session, &nav_params)
                .await
                .map_err(map_transport_err)?;

            // Keep the legacy bounded settle delay, but no persistent browser-side registration
            // now exists if the future is cancelled during it.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let mini_state = crate::storage_state::StorageState {
                cookies: vec![],
                local_storage: vec![origin_storage.clone()],
            };
            if !origin_storage.local_storage.is_empty() {
                self.restore_local_storage(&mini_state).await?;
            }
            if origin_storage.index_db.is_some() {
                self.restore_index_db(&mini_state).await?;
            }
        }

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// D3：tab 动作（tabs / switch_tab / close_tab / open_link_new_tab）。
// 全程沿用 active_tab_handles 的锁模式——短临界区锁 `tabs`/`active_target` 取/改后立即释放，
// **绝不**跨 await 持 `tabs` 锁（CDP 命令的 await 都在锁外）。switch 是纯逻辑指针切换
// （正确性不依赖 activateTarget/bringToFront，headless 弱——DESIGN:213/不变量⑱）。
// ═══════════════════════════════════════════════════════════════════════════

impl CdpBackend {
    /// **[纯逻辑/锁内] 在 `tabs` 注册表里按 last4 / 全 id 解析一个 tab_id**：短暂锁 `tabs` 取所有 key →
    /// [`crate::tabs::resolve_last4_among`] 判唯一/撞号/零命中 → 唯一返完整 targetId；撞号 → `Blocked`
    /// （让 LLM 用更长前缀）；零命中 → `Blocked`（无此 tab）。**不进浏览器**（只查注册表）。
    async fn resolve_tab_id(&self, tab_id: &str) -> Result<String, BrowserError> {
        use crate::tabs::{resolve_last4_among, Last4Match};
        let ids: Vec<String> = {
            let guard = self.tabs.lock().await;
            guard.keys().cloned().collect()
        };
        match resolve_last4_among(tab_id, ids.iter().map(|s| s.as_str())) {
            Last4Match::Unique(full) => Ok(full),
            Last4Match::Ambiguous(hits) => Err(BrowserError::Blocked {
                reason: format!(
                    "tab id {tab_id:?} is ambiguous (matches {}); use a longer id",
                    hits.len()
                ),
            }),
            Last4Match::NotFound => Err(BrowserError::Blocked {
                reason: format!("no open tab matches id {tab_id:?}; call tabs to list open tabs"),
            }),
        }
    }

    /// Structured top-level tab inventory used by non-LLM platform callers.
    /// Registry ownership is authoritative; `Target.getTargets` contributes
    /// only display metadata.
    async fn structured_tab_inventory(&self) -> Result<Vec<BrowserTabInfo>, BrowserError> {
        use crate::tabs::last4;
        use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;

        let (managed, active): (Vec<(String, String)>, String) = {
            let tabs = self.tabs.lock().await;
            let managed = tabs
                .iter()
                .map(|(target_id, record)| {
                    (target_id.clone(), record.session_id.clone())
                })
                .collect();
            let active = self.active_target.lock().await.clone();
            (managed, active)
        };

        let mut display: HashMap<String, (String, String)> = HashMap::new();
        if let Ok(raw) = self
            .conn
            .send::<GetTargetsParams>(ROOT_SESSION, &GetTargetsParams { filter: None })
            .await
            && let Some(targets) = raw.get("targetInfos").and_then(|value| value.as_array())
        {
            for target in targets {
                let Some(target_id) = target.get("targetId").and_then(|value| value.as_str())
                else {
                    continue;
                };
                let url = target
                    .get("url")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned();
                let title = target
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned();
                display.insert(target_id.to_owned(), (url, title));
            }
        }

        let mut tabs = managed
            .into_iter()
            .map(|(target_id, session_id)| {
                let (url, title) = display.get(&target_id).cloned().unwrap_or_default();
                BrowserTabInfo {
                    tab_id: last4(&target_id),
                    active: target_id == active,
                    crashed: self.conn.registry().is_session_crashed(&session_id),
                    target_id,
                    title: (!title.is_empty()).then_some(title),
                    url: (!url.is_empty()).then_some(url),
                }
            })
            .collect::<Vec<_>>();
        tabs.sort_by(|left, right| {
            left.tab_id
                .cmp(&right.tab_id)
                .then_with(|| left.target_id.cmp(&right.target_id))
        });
        Ok(tabs)
    }

    /// **tabs 列表动作**（D3，DESIGN §13，Info 级只读）：枚举当前所有纳管标签 → (last4, url, title,
    /// is_active) → 渲染成对 LLM 文案。url/title 经 `Target.getTargets`（一次取全量 targetInfo），按本
    /// 注册表的 tab key 过滤（只列我们纳管的 page，不含 OOPIF/SW/其它 browser target）。
    pub async fn act_tabs(&self) -> Result<ActResult, BrowserError> {
        use crate::tabs::{TabListItem, render_tab_list};

        let items: Vec<TabListItem> = self
            .structured_tab_inventory()
            .await?
            .into_iter()
            .map(|tab| TabListItem {
                last4: tab.tab_id,
                target_id: tab.target_id,
                url: tab.url.unwrap_or_default(),
                title: tab.title.unwrap_or_default(),
                is_active: tab.active,
            })
            .collect();

        Ok(ActResult {
            message: render_tab_list(&items),
            effect: Effect {
                changed: false,
                before_anchor: None,
                after_anchor: None,
            },
            success: true,
        })
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 调试捕获读取动作（只读，读 per-tab 缓冲并脱敏序列化）
    // ═══════════════════════════════════════════════════════════════════════

    /// `get_console_logs` 动作：读取 active tab 的 console 缓冲，脱敏后返回。
    pub async fn act_get_console_logs(&self) -> Result<ActResult, BrowserError> {
        let handles = self.active_tab_handles().await?;
        let snap = crate::debug_capture::DebugSnapshot::from_buffers(&handles.debug);
        let message = self.known_secret_values.with_values(|secrets| {
            crate::debug_capture::serialize_console_for_llm(&snap.console, secrets)
        });
        Ok(ActResult {
            message,
            effect: Effect { changed: false, before_anchor: None, after_anchor: None },
            success: true,
        })
    }

    /// `get_page_errors` 动作：读取 active tab 的 errors 缓冲，脱敏后返回。
    pub async fn act_get_page_errors(&self) -> Result<ActResult, BrowserError> {
        let handles = self.active_tab_handles().await?;
        let snap = crate::debug_capture::DebugSnapshot::from_buffers(&handles.debug);
        let message = self.known_secret_values.with_values(|secrets| {
            crate::debug_capture::serialize_errors_for_llm(&snap.errors, secrets)
        });
        Ok(ActResult {
            message,
            effect: Effect { changed: false, before_anchor: None, after_anchor: None },
            success: true,
        })
    }

    /// `get_network_log` 动作：读取 active tab 的 network 缓冲，脱敏后返回。
    pub async fn act_get_network_log(&self, include_bodies: bool) -> Result<ActResult, BrowserError> {
        let handles = self.active_tab_handles().await?;
        let snap = crate::debug_capture::DebugSnapshot::from_buffers(&handles.debug);
        let message = self.known_secret_values.with_values(|secrets| {
            crate::debug_capture::serialize_network_for_llm(&snap.network, include_bodies, secrets)
        });
        Ok(ActResult {
            message,
            effect: Effect { changed: false, before_anchor: None, after_anchor: None },
            success: true,
        })
    }

    /// **switch_tab 动作**（D3，DESIGN §13/不变量⑱）：解析 tab_id（撞号→Blocked）→ 把 `active_target`
    /// 指向该 targetId（**逻辑指针切换**）。切换后 observe/act 自动作用新 active tab（经 active_tab_handles）。
    /// headful 下**额外** best-effort `Target.activateTarget` 把它前置（**正确性不依赖它**——headless 弱）。
    pub async fn act_switch_tab(&self, tab_id: &str) -> Result<ActResult, BrowserError> {
        use crate::tabs::last4;
        use chromiumoxide::cdp::browser_protocol::target::ActivateTargetParams;

        let target_id = self.resolve_tab_id(tab_id).await?;
        // 逻辑指针切换（短临界区）。
        {
            let mut active = self.active_target.lock().await;
            *active = target_id.clone();
        }
        // headful best-effort 前置（不影响正确性；headless 弱，失败忽略）。
        if self.headful {
            let _ = self
                .conn
                .send::<ActivateTargetParams>(ROOT_SESSION, &ActivateTargetParams::new(target_id.clone()))
                .await;
        }
        let l4 = last4(&target_id);
        Ok(ActResult {
            message: format!("switched to tab [{l4}]; observe to see its content"),
            effect: Effect {
                changed: true,
                before_anchor: None,
                after_anchor: Some(serde_json::json!({ "active_tab": l4 })),
            },
            success: true,
        })
    }

    /// **close_tab 动作**（D3，DESIGN §13）：解析 tab_id → `Target.closeTarget` → 从 `tabs` 移除
    /// 该 [`TabRecord`] → **显式 `.abort()` 其 `_inject_loop`/`_oopif_loop`**（D1 评审要点：drop 是
    /// detach 非 abort，全局连接仍存活时这俩循环不会靠 `RecvError::Closed` 退出，必须显式 abort 防泄漏
    /// 空转）→ 若关的是 active tab：重选一个剩余 tab 作 active（无剩余 → `SessionLost`，并对进行中操作
    /// `Progress::abort(PageClosed)`）。
    ///
    /// `parent` 是动作的 [`Progress`] 作用域：关掉 active tab 时若有进行中操作绑该 tab，本动作的 parent
    /// 取消会经 token 层级传播（这里对 parent abort(PageClosed) 兑现「关 active → 进行中操作立即取消」）。
    pub async fn close_tab_impl(
        &self,
        tab_id: &str,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        use crate::tabs::last4;

        let target_id = self.resolve_tab_id(tab_id).await?;
        let l4 = last4(&target_id);

        // A successful command response is not assumed to mean the renderer
        // disappeared: closeTarget success=false/malformed/error is accepted
        // only after a root inventory proves exact absence.
        if let Err(error) = close_target_or_confirm_absent(&self.conn, &target_id).await {
            if let Some(host) = &self.host {
                host.router
                    .schedule_owned_target_cleanup(&self.lane_id, &target_id)
                    .await;
            } else {
                PendingCreatedPageCleanup::new(
                    self.conn.clone(),
                    Arc::clone(&self.cleanup_executor),
                    None,
                    Some(target_id.clone()),
                    None,
                    None,
                    None,
                    None,
                )
                .hand_off();
            }
            return Err(error);
        }

        // 2) 从 tabs 移除 TabRecord + **显式 abort 其两个后台循环**（防泄漏空转）。短临界区。
        let was_active;
        let reselected: Option<String>;
        let mut removed_main_frame = None;
        {
            let mut tabs = self.tabs.lock().await;
            if let Some(record) = tabs.remove(&target_id) {
                removed_main_frame = Some(record.main_frame_id.clone());
                // D1 评审要点：drop(record) 只 detach JoinHandle，全局连接仍存活时循环不退出 → 必须显式 abort。
                record._inject_loop.abort();
                record._oopif_loop.abort();
                record._debug_loop.abort();
            }
            // 3) 若关的是 active tab：重选一个剩余 tab 作 active。
            let mut active = self.active_target.lock().await;
            was_active = *active == target_id;
            if was_active {
                // 取任一剩余 tab（按 key 排序取最小，确定性）。
                reselected = tabs.keys().min().cloned();
                if let Some(ref new_active) = reselected {
                    *active = new_active.clone();
                } else {
                    active.clear();
                }
            } else {
                reselected = None;
            }
        }
        if let Some(host) = &self.host {
            host.router
                .release_target(&target_id, removed_main_frame.as_deref())
                .await;
        }

        if was_active {
            // 关 active：对进行中操作（绑该 tab 的 act）发 abort(PageClosed)——立即取消而非白等。
            parent.abort(AbortReason::PageClosed);
            match reselected {
                Some(new_id) => {
                    let new_l4 = last4(&new_id);
                    Ok(ActResult {
                        message: format!(
                            "closed active tab [{l4}]; active is now [{new_l4}] (observe to see it)"
                        ),
                        effect: Effect {
                            changed: true,
                            before_anchor: None,
                            after_anchor: Some(serde_json::json!({ "active_tab": new_l4 })),
                        },
                        success: true,
                    })
                }
                // 关了最后一个 tab：没有可作用的 page 了（session lost）。
                None => Err(BrowserError::SessionLost { recoverable: true }),
            }
        } else {
            // 关的是非 active tab：active 不变，本动作不影响进行中操作。
            Ok(ActResult {
                message: format!("closed tab [{l4}]; active tab unchanged"),
                effect: Effect {
                    changed: true,
                    before_anchor: None,
                    after_anchor: None,
                },
                success: true,
            })
        }
    }

    /// **open_link_new_tab 动作**（D3，DESIGN §13）：`Target.createTarget{url, background:true}`——
    /// **background 不抢焦点**。新 page 的 attachedToTarget 会被 [`spawn_tab_discovery_loop`] 收编 arm
    /// 入 `tabs`（不改 active）。返回新 tab 的 last4（让 LLM 显式 switch）。
    ///
    /// **active 不变**：本动作只开 tab、不切换；observe/act 仍作用原 active tab，直到 LLM 显式 switch_tab。
    pub async fn act_open_link_new_tab(&self, url: &str) -> Result<ActResult, BrowserError> {
        use crate::tabs::last4;

        let tab_count = self.tabs.lock().await.len();
        if !tab_capacity_available(tab_count) {
            return Err(BrowserError::Blocked {
                reason: format!(
                    "this browser Lane already has {MAX_TABS_PER_LANE} tabs; close an existing tab before opening another"
                ),
            });
        }

        // Create an inert nonce-correlated page first. This makes cancellation
        // and a lost createTarget response exactly recoverable; only after the
        // target is lane-owned/armed do we navigate it to the requested URL.
        let pending_page = create_pending_page_session_owned(
            self.conn.clone(),
            Arc::clone(&self.cleanup_executor),
            self.target_router().cloned(),
            true,
            self.task_tab_reservation_scope.clone(),
        )
        .await?;
        let new_tid = pending_page.target_id.clone();
        let new_session = pending_page.session_id.clone();
        if let Some(host) = &self.host {
            if !host.router.claim_target(&self.lane_id, &new_tid).await {
                return Err(BrowserError::TargetCrashed);
            }
        }
        if !self.tabs.lock().await.contains_key(&new_tid) {
            let record = arm_tab(
                &self.conn,
                &new_tid,
                &new_session,
                pending_page.task_tab_reservation.clone(),
            )
            .await?;
            if let Some(host) = &self.host {
                let outcome = host
                    .router
                    .publish_armed_page(
                        &self.lane_id,
                        PendingPage {
                            target_id: new_tid.clone(),
                            session_id: new_session.clone(),
                            opener_target_id: None,
                            target_url: None,
                        },
                        record,
                    )
                    .await;
                if outcome == OwnedPagePublish::RejectedCapacity {
                    let _ = pending_page.transfer_to_lane();
                    return Err(BrowserError::Blocked {
                        reason: "this task reached its browser tab limit; close an existing tab before opening another".into(),
                    });
                }
                if outcome == OwnedPagePublish::RejectedState {
                    return Err(BrowserError::TargetClosed);
                }
            } else {
                let mut tabs = self.tabs.lock().await;
                if !tabs.contains_key(&new_tid) && tab_capacity_available(tabs.len()) {
                    tabs.insert(new_tid.clone(), record);
                } else {
                    abort_tab_record(&record);
                }
            }
        }
        if let Err(error) = self
            .conn
            .send::<NavigateParams>(&new_session, &NavigateParams::new(url))
            .await
        {
            // F53：standalone（无 router）后端没有 target-loss 事件路径来移除刚 arm
            // 的 TabRecord——navigate 失败时 PendingCreatedPage 的清理会关掉 target，
            // 但 tabs 里的记录会残留成永久幽灵 tab（tabs 恒列出、switch 后一切操作
            // TargetClosed）。此处主动剔除并 abort 其后台循环。host 模式保留原
            // loss 事件路径（detach → handle_top_level_target_loss 同步移除记录并
            // 清 frame_owner），不在此双改。
            if self.target_router().is_none()
                && let Some(record) = self.tabs.lock().await.remove(&new_tid)
            {
                abort_tab_record(&record);
            }
            return Err(map_transport_err(error));
        }
        let _ = pending_page.transfer_to_lane();
        let l4 = last4(&new_tid);

        Ok(ActResult {
            message: format!(
                "opened {url} in a new tab [{l4}] (did not switch to it); use switch_tab [{l4}] to focus it"
            ),
            effect: Effect {
                // 开了新 tab（页面态有变），但 active tab 未变。
                changed: true,
                before_anchor: None,
                after_anchor: Some(serde_json::json!({ "new_tab": l4 })),
            },
            success: true,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// D4：history 导航（back/forward）+ reload（POST 页→IRREVERSIBLE 检测）+ switch_frame
// （active_frame 逻辑指针）。全经 active tab 句柄；settle **复用 D2 的 settle_after_trigger
// → run_settle**（零分叉，对齐 D4「别另写一套 settle」）；良性边界态（无更多历史）不报错。
// ═══════════════════════════════════════════════════════════════════════════

impl CdpBackend {
    /// **back/forward 动作**（D4，DESIGN §12）：`Page.getNavigationHistory` 取 currentIndex + entries
    /// → [`nav::history_target_index`] 算目标 entry 索引（**边界钳制**：首页 back / 末页 forward →
    /// `None`，**良性返「无更多历史」success=true 不报错、不 panic**）→ `Page.navigateToHistoryEntry`
    /// → **复用 D2 settle**（[`Self::settle_after_trigger`] → run_settle）等导航完成 → 返 [`NavResult`]。
    /// 经 active tab 的 page session + 主帧句柄。
    pub async fn act_history_nav(
        &self,
        direction: nav::HistoryNav,
    ) -> Result<ActResult, BrowserError> {
        use chromiumoxide::cdp::browser_protocol::page::GetNavigationHistoryParams;

        let handles = self.active_tab_handles().await?;
        let session = handles.session_id.clone();
        let main_frame_id = handles.main_frame_id.clone();

        // 取导航历史（currentIndex + entries）。失败 → 上抛（传输错）。
        let history = self
            .conn
            .send::<GetNavigationHistoryParams>(&session, &GetNavigationHistoryParams::default())
            .await
            .map_err(map_transport_err)?;
        let current_index = history
            .get("currentIndex")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                BrowserError::Other("getNavigationHistory missing currentIndex".into())
            })?;
        let entries = history
            .get("entries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let dir_label = match direction {
            nav::HistoryNav::Back => "back",
            nav::HistoryNav::Forward => "forward",
        };

        // 边界钳制：算目标 entry 索引（首页 back / 末页 forward → None = 良性无更多历史）。
        let Some(target_idx) = nav::history_target_index(current_index, entries.len(), direction)
        else {
            // 良性：无更多历史 → success=true（changed=false），如实告知，**不报错、不 panic**。
            return Ok(ActResult {
                message: format!("no more history to go {dir_label} (already at the edge)"),
                effect: Effect {
                    changed: false,
                    before_anchor: None,
                    after_anchor: None,
                },
                success: true,
            });
        };

        // 取目标 entry 的 id（navigateToHistoryEntry 按 entryId 导航）。形状异常 → Other（不 panic）。
        let entry_id = entries
            .get(target_idx)
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                BrowserError::Other(format!(
                    "navigation history entry {target_idx} missing id"
                ))
            })?;

        // 触发前的当前 url（redirect 归一化比较的「from」端——history 导航回到的目标即「应到」的页，
        // 与 final_url 同源 → 一般不算 redirect）。
        let before_url = self.current_url(&session).await.unwrap_or_default();

        // navigateToHistoryEntry + **复用 D2 settle**（settle_after_trigger → run_settle，零分叉）。
        let nav = self
            .settle_after_trigger(&session, &main_frame_id, &before_url, |conn, sess| async move {
                use chromiumoxide::cdp::browser_protocol::page::NavigateToHistoryEntryParams;
                conn.send::<NavigateToHistoryEntryParams>(
                    &sess,
                    &NavigateToHistoryEntryParams::new(entry_id),
                )
                .await
                .map_err(map_transport_err)?;
                Ok(())
            })
            .await?;

        Ok(ActResult {
            message: format!(
                "navigated {dir_label}; now at {} (load state: {}); re-observe to see the page",
                nav.final_url, nav.load_state
            ),
            effect: Effect {
                changed: true,
                before_anchor: Some(serde_json::Value::String(before_url)),
                after_anchor: Some(serde_json::json!({
                    "url": nav.final_url,
                    "load_state": nav.load_state.to_string(),
                    "http_status": nav.http_status,
                })),
            },
            success: true,
        })
    }

    /// **reload 动作**（D4，DESIGN §12 + 裁决⑧）：`Page.reload`（`ignoreCache=false`，普通刷新）→
    /// **复用 D2 settle**（[`Self::settle_after_trigger`] → run_settle）。
    ///
    /// **POST 页 → IRREVERSIBLE 检测（裁决⑧，D4 只检测/标记，不接 enforcement）**：reload 前查
    /// `Page.getNavigationHistory`，若当前 entry 的 `transitionType == form_submit`（POST 表单提交页，
    /// reload 会**重新提交表单**——重复下单/扣款/发消息），[`nav::current_entry_is_post`] 判 true，则：
    /// - 记一条 `tracing::warn` 标记（与 C2 press_key 的 IRREVERSIBLE 检测同范式）；
    /// - 在返回的 [`ActResult`] `effect.after_anchor` 带 `irreversible: true` 标志（供 E2/F1 接强制门读）；
    /// - **不**在此 enforce（不 hard-deny / 不要带外确认）——强制门是 E2/F1 的 facade 独立门职责。
    ///
    /// **可行性说明**：CDP 不直接给导航的 HTTP method；`transitionType==form_submit` 是最接近的可观测
    /// 信号（PW/browser-use 同此）。GET 表单也标 form_submit 会被**保守过判**为 irreversible（宁多确认
    /// 不漏判真 POST）；拿不到 transition（缺字段/形状陌生）→ **保守判非 irreversible**（不给普通页 reload
    /// 加确认门，与 spec「拿不准时不误判」一致）。
    pub async fn act_reload(&self) -> Result<ActResult, BrowserError> {
        use chromiumoxide::cdp::browser_protocol::page::{
            GetNavigationHistoryParams, ReloadParams,
        };

        let handles = self.active_tab_handles().await?;
        let session = handles.session_id.clone();
        let main_frame_id = handles.main_frame_id.clone();

        // POST 页检测（裁决⑧，D4 只检测）：查导航历史当前 entry 的 transitionType。best-effort——取
        // 不到历史 → 保守 false（不误判普通页 reload 为不可逆）。
        let irreversible = match self
            .conn
            .send::<GetNavigationHistoryParams>(&session, &GetNavigationHistoryParams::default())
            .await
        {
            Ok(history) => {
                let current_index = history.get("currentIndex").and_then(|v| v.as_i64()).unwrap_or(-1);
                let entries = history.get("entries").cloned().unwrap_or(serde_json::Value::Null);
                nav::current_entry_is_post(&entries, current_index)
            }
            Err(_) => false,
        };
        if irreversible {
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                "reload detected IRREVERSIBLE (current page came from a POST form submit; \
                 reloading re-submits the form); TODO(E2/F1): wire fail-closed enforcement \
                 (D4 detection-only, not blocking)"
            );
        }

        let before_url = self.current_url(&session).await.unwrap_or_default();

        // Page.reload（ignoreCache=false 普通刷新）+ **复用 D2 settle**（settle_after_trigger →
        // run_settle）。redirect 比较「from」用触发前 url（reload 通常停在同 url → 不算 redirect）。
        let nav = self
            .settle_after_trigger(&session, &main_frame_id, &before_url, |conn, sess| async move {
                let params = ReloadParams::builder().ignore_cache(false).build();
                conn.send::<ReloadParams>(&sess, &params)
                    .await
                    .map_err(map_transport_err)?;
                Ok(())
            })
            .await?;

        let message = if irreversible {
            format!(
                "reloaded {} (load state: {}); NOTE: this page came from a form submission — \
                 reloading may re-submit it (irreversible); re-observe to see the page",
                nav.final_url, nav.load_state
            )
        } else {
            format!(
                "reloaded {} (load state: {}); re-observe to see the page",
                nav.final_url, nav.load_state
            )
        };

        Ok(ActResult {
            message,
            effect: Effect {
                changed: true,
                before_anchor: Some(serde_json::Value::String(before_url)),
                after_anchor: Some(serde_json::json!({
                    "url": nav.final_url,
                    "load_state": nav.load_state.to_string(),
                    "http_status": nav.http_status,
                    // IRREVERSIBLE 标志（D4 只标记；E2/F1 facade 独立门读它做强制门）。
                    "irreversible": irreversible,
                })),
            },
            success: true,
        })
    }

    /// **switch_frame 动作**（D4，DESIGN §13 语义采纳见下）：把后续**页面级动作**的默认作用域从主帧
    /// 切到给定 iframe 元素 ref 指向的**内容帧**（content frame）。
    ///
    /// **采纳的 DESIGN §13 语义**：DESIGN §13 把 `switch_frame` 列为导航类动作但未细化语义；§9 列其为
    /// Info/Exec。结合 PLAN D4「switch_frame（ref→frame session/world）」+ §7 句柄模型（ref-based 动作
    /// 经 `aria-ref=f<seq>e<n>` 本就跨帧工作、不受 active_frame 影响），最自洽的实现 = 设一个
    /// **active_frame 逻辑指针**（`Some((session_id, frame_id))` = 已切入某 iframe / `None` = 主帧/顶层），
    /// 让**页面级动作**（get_page_text / scroll(viewport) / press_key / find_elements 等无 element ref 的
    /// 动作）默认作用于该 iframe 而非主帧；ref-based 动作（click/type 经 ref 前缀路由到所属帧）**不受
    /// 影响**（本就跨帧）。这与 browser-use 的 switch_frame 语义一致（聚焦 LLM 的「当前帧」上下文）。
    ///
    /// **解析**：resolve ref（层①②③，须是页面里的 iframe 元素）→ 取其元素 objectId → `DOM.describeNode`
    /// 读该 iframe 元素的 `node.frameId`（= **内容帧** id；iframe 元素 node 的 frameId 即它承载的子文档帧）
    /// → 设 active_frame 指针为 `(该 iframe 所属 session, contentFrameId)`。非 iframe 元素（describeNode
    /// 无 frameId）→ success=false 如实（引导换 ref）。
    ///
    /// **切回主帧/顶层**：传特殊 ref `"main"` / `"top"` / 空串 → active_frame 置 `None`（页面级动作回主帧）。
    ///
    /// **D4 范围**：设指针 + 让 active_frame 影响**页面级动作**的 frame 解析（见 [`Self::active_page_frame`]
    /// 接入点）。注：D4 把指针接进 [`Self::main_frame_id`]/[`Self::page_session_id`] 的**页面级解析**
    /// （这俩是页面级动作取 frame 的入口）；同进程 iframe（同 page session）已可端到端验证，跨进程 OOPIF
    /// 切帧接线就位但离线 fixture 触发不到（`TODO(verify-oopif)`）。
    pub async fn act_switch_frame(&self, llm_ref: &str) -> Result<ActResult, BrowserError> {
        // 切回主帧/顶层：特殊 ref（main/top/空）→ active_frame 置 None。
        let trimmed = llm_ref.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("main") || trimmed.eq_ignore_ascii_case("top") {
            {
                let mut af = self.active_frame.lock().await;
                *af = None;
            }
            return Ok(ActResult {
                message: "switched back to the top/main frame; page-level actions now act on the main document".into(),
                effect: Effect {
                    changed: true,
                    before_anchor: None,
                    after_anchor: Some(serde_json::json!({ "active_frame": "main" })),
                },
                success: true,
            });
        }

        // resolve ref（层①②③）：拿活元素句柄 + 它所属帧（iframe 元素本身住在父帧）。
        let seq = self.next_act_seq();
        let rec = self.resolve_ref_record(llm_ref).await?;
        let handle = self.resolve_ref_to_object(&rec, seq).await?;

        // describeNode 读 iframe 元素的内容帧 id（node.frameId）。iframe 元素 node 的 frameId 即其承载的
        // 子文档帧；非 iframe 元素 → 无 frameId → 良性失败（引导换 ref）。在 iframe 元素所属 session 上发。
        let content_frame_id = self.iframe_content_frame_id(&rec.session_id, &handle.object_id).await;
        // 释放本次 resolve 的句柄组（switch_frame 不持续持有元素句柄）。
        self.release_act_group(&rec, seq).await;

        let Some(content_frame_id) = content_frame_id else {
            // 非 iframe 元素（无 contentFrame）→ success=false 如实（非报错，良性，引导换 ref）。
            return Ok(ActResult {
                message: format!(
                    "{llm_ref} is not an <iframe> (no content frame); switch_frame only works on iframe elements"
                ),
                effect: Effect {
                    changed: false,
                    before_anchor: None,
                    after_anchor: None,
                },
                success: false,
            });
        };

        // 设 active_frame 指针：页面级动作据此把默认作用域切到该 iframe（session 沿用 iframe 元素所属
        // session——同进程 iframe 与父帧同 page session；跨进程 OOPIF 子帧另起子 session，离线测不到）。
        {
            let mut af = self.active_frame.lock().await;
            *af = Some((rec.session_id.clone(), content_frame_id.clone()));
        }

        Ok(ActResult {
            message: format!(
                "switched into iframe {llm_ref}; page-level actions (get_page_text/scroll/find_elements/…) \
                 now act on that frame; re-observe to see its content"
            ),
            effect: Effect {
                changed: true,
                before_anchor: None,
                after_anchor: Some(serde_json::json!({ "active_frame": content_frame_id })),
            },
            success: true,
        })
    }

    /// **[运行时] 读一个 iframe 元素的内容帧 frameId**（switch_frame 解析）：`DOM.describeNode{objectId}`
    /// 返回该元素的 Node 描述，其中 `node.frameId` 对 **iframe 元素**即它承载的子文档帧 id。在元素所属
    /// `session` 上发。非 iframe 元素 → Node 无 frameId → `None`（良性，调用方引导换 ref）。任何 CDP/形状
    /// 失败 → `None`（best-effort，绝不 panic）。
    async fn iframe_content_frame_id(&self, session: &str, object_id: &str) -> Option<String> {
        use chromiumoxide::cdp::browser_protocol::dom::DescribeNodeParams;
        let params = DescribeNodeParams::builder()
            .object_id(RemoteObjectId::new(object_id.to_string()))
            .build();
        let result = self
            .conn
            .send::<DescribeNodeParams>(session, &params)
            .await
            .ok()?;
        // node.frameId（iframe 元素 → 内容帧 id；非 iframe → 缺该字段）。
        result
            .get("node")
            .and_then(|n| n.get("frameId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// **[运行时] 页面级动作的当前作用帧**（switch_frame 接入点）：若 active_frame 指针指向某 iframe
    /// （switch_frame 切入后），返该 iframe 的 `(session_id, frame_id)`；否则（`None` / 指向的帧已不在
    /// 当前 active tab）退到 active tab 的主帧 `(page_session, main_frame_id)`。
    ///
    /// **页面级动作**（get_page_text / scroll(viewport) / press_key / find_elements / cursor / scroll_to_text
    /// 等无 element ref 的动作）经此取作用帧——故 switch_frame 后它们作用于 iframe 而非主帧。**ref-based
    /// 动作不经此**（它们按 ref 的所属帧路由，本就跨帧，不受 active_frame 影响）。
    ///
    /// active_frame 指向的 frame 不再属于当前 active tab（切了 tab / 帧已 detach）→ 退主帧（保守，
    /// 不在错的 tab 上操作 stale 帧）。
    pub(crate) async fn active_page_frame(&self) -> Result<(String, String), BrowserError> {
        let handles = self.active_tab_handles().await?;
        let af = self.active_frame.lock().await.clone();
        if let Some((session, frame_id)) = af {
            // active_frame 的 session 必须是当前 active tab 的 page session（同进程 iframe）或其 OOPIF
            // 子 session——D4 范围：同进程 iframe 与主帧同 page session，故校验 session 匹配 active tab。
            // 跨 tab 切换后旧 active_frame 失效 → 退主帧。
            if session == handles.session_id {
                return Ok((session, frame_id));
            }
            // 也可能是 active tab 的 OOPIF 子 session（跨进程子帧；离线测不到，接线就位）。
            if handles
                .oopif_managers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(&session)
            {
                return Ok((session, frame_id));
            }
            // 否则（切了 tab / 帧 detach）→ 退主帧（保守）。
        }
        Ok((handles.session_id, handles.main_frame_id))
    }

    /// **[运行时] 在「当前作用帧」的 world 里跑一段只读 `Runtime.evaluate`（by-value）**（D4 switch_frame
    /// 接入点：页面级只读动作经此取文本/状态，故 switch_frame 后作用于 iframe 而非主帧）。
    ///
    /// 作用帧由 [`Self::active_page_frame`] 决定：
    /// - **未 switch_frame（active_frame=None）/ 已退主帧**：在该 (page session) 的**默认 page world**
    ///   `evaluate`（无 context_id——与 D4 前行为完全一致，主帧 document）。
    /// - **已 switch_frame 切入 iframe**：在该 iframe 的 **utility-world contextId** `evaluate`
    ///   （isolated world 与页面 world 共享同一 DOM document，故 `document.body.innerText`/`location.href`
    ///   等读到的是**该 iframe 的文档**）。utility context 未就绪（导航中）→ 退默认 world（best-effort）。
    ///
    /// 返回 `result.result.value`（by-value）；JS 抛异常 → `Err(Other)`；CDP 失败 → 经 map_transport_err。
    /// 抽此单点让所有页面级只读 helper（get_page_text/scroll_to_text/text_present/count_pointer_cursor/
    /// focus_in_form/current_url）一致地受 active_frame 影响（switch_frame 一处改、全页面级动作生效）。
    pub(crate) async fn active_frame_eval(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, BrowserError> {
        let (session, frame_id) = self.active_page_frame().await?;
        let mut params = EvaluateParams::new(expression.to_string());
        params.return_by_value = Some(true);
        params.await_promise = Some(false);
        // 若 active_frame 指向非主帧 iframe，且其 utility context 已就绪 → 在该 context evaluate
        // （作用于 iframe 文档）。主帧 / context 未就绪 → 默认 page world（无 context_id，主文档）。
        let main_frame_id = self.active_tab_handles().await?.main_frame_id;
        if frame_id != main_frame_id
            && let Ok(injection) = self.injection_manager().await
            && let Ok(ctx) = injection.context_id_for(&frame_id)
        {
            params.context_id = Some(ExecutionContextId::new(ctx));
        }
        let result = self
            .conn
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(BrowserError::Other(format!("eval threw: {ex}")));
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// **Evaluate 分支**（E3，DESIGN §16「evaluate」/ 裁决⑨）：在页面上下文跑任意 JS——**最高危逃生舱**，
    /// 故是引擎默认最高门控。
    ///
    /// 门控逻辑（[`crate::evaluate::gate`]，**纯逻辑、不读 session_mode**——不变量⑧）：
    /// - **默认 OFF**：未显式开「全权模式」→ [`crate::evaluate::evaluate_off_error`]（`Unsupported{evaluate}`，
    ///   hint 讲清为何 off + 怎么开）。这是 default-deny——没有任何 session 默认能 evaluate。
    /// - **opt-in 全权**：用户显式 opt-in 全权（LIVE 读，F1 灌入 [`Self::evaluate_gate`]）+ 无持久登录 → 放行。
    /// - **与持久登录互斥**：全权 + 持久登录同开 → `Blocked`（互斥；持久登录灌着真实长期凭据，禁任意 JS）。
    /// - **持久登录下封死**：持久登录开启时 evaluate 强制 OFF（即便全权也被互斥拦下）。
    /// - **yolo 不豁免**：放行**只看全权开关**，不看 `SessionMode`——yolo/companion 无从豁免（不变量⑧）。
    ///
    /// 放行后：记一条**醒目审计**（[`crate::evaluate::audit_evaluate`]，script 只记**脱敏摘要**不记全文）→
    /// 在当前作用帧 [`Self::active_frame_eval`] 跑该脚本，返 by-value 结果。**前端醒目展示留 P3**。
    ///
    /// **绝不自动重试**（裁决⑧/⑨「IRREVERSIBLE 禁重试」镜像）：evaluate 不走 [`run_act_with_retry`] 退避，
    /// 单次执行——任意 JS 副作用不可逆，重试可能重复执行。
    pub(crate) async fn act_evaluate(&self, script: &str) -> Result<ActResult, BrowserError> {
        // 门控（纯逻辑，**不读 session_mode**）：默认 OFF / 全权 opt-in / 与持久登录互斥。
        let cfg = *self.evaluate_gate.lock().await;
        crate::evaluate::gate(&cfg)?;

        // 放行（仅全权 opt-in + 无持久登录）：记醒目审计（script 只记脱敏摘要不记全文）。
        let origin = self.act_current_url().await;
        crate::evaluate::audit_evaluate(script, origin.as_deref());

        // 在当前作用帧跑该脚本（单次，绝不重试）。JS 抛异常 → Err(Other)；CDP 失败 → 映射错误。
        let value = self.active_frame_eval(script).await?;
        Ok(ActResult {
            message: format!(
                "evaluated script in the page (full-power mode); result: {value}; re-observe to see any DOM changes"
            ),
            effect: Effect {
                // evaluate 任意 JS 可能改 DOM/导航——保守视作 changed（无法静态判定其副作用）。
                changed: true,
                before_anchor: None,
                after_anchor: Some(value),
            },
            success: true,
        })
    }

    // ═══════════════════════════════════════════════════════════════════════
    // F-actions CDP 原语（pub(crate)，供 actions.rs 的 act_upload_file / act_download /
    // act_save_as_pdf 编排调用）。这些方法持有 self.conn / self.download_dir / active tab 句柄等
    // **cdp 模块私有态**的访问权，故落在 cdp.rs；编排逻辑（skeleton/retry/RetryDecision）在 actions.rs。
    // ═══════════════════════════════════════════════════════════════════════

    /// **隔离下载目录绝对路径访问器**（F-actions：download 探测落点 / save_as_pdf 写入落点）。
    /// `Some` 当且仅当 E4 下载沙箱已挂；`None` = 纯引擎冒烟（无落点）。
    pub(crate) fn download_dir(&self) -> Option<&str> {
        self.download_dir.as_deref()
    }

    pub(crate) async fn reserve_direct_download_output(
        &self,
        kind: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError> {
        static NEXT_DIRECT_DOWNLOAD_ID: AtomicU64 = AtomicU64::new(1);
        let scope = self
            .task_download_reservation_scope
            .as_ref()
            .ok_or_else(|| BrowserError::Blocked {
                reason: "this browser Lane has no task-lifetime download authority".into(),
            })?;
        let nonce = NEXT_DIRECT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed);
        scope.reserve(&format!("direct-{kind}-{nonce}")).await
    }

    /// **SD-2 上传路径沙箱根访问器**。`Some` = per-pet workspace（upload 必须 in-sandbox）；
    /// `None` = 无 workspace（fail-closed：一律拒绝上传）。
    pub(crate) fn workspace_dir(&self) -> Option<&std::path::Path> {
        self.workspace_dir.as_deref()
    }

    /// **[运行时] DOM.setFileInputFiles（在 file input 元素上设置上传文件路径）**（upload_file 真执行）。
    /// `object_id` 是 resolve_ref 产出的 utility-world 元素句柄——DOM 域按 objectId 解析节点（跨 world，
    /// 与 [`Self::iframe_content_frame_id`] 的 describeNode 同范式）。元素不是 `<input type=file>` →
    /// CDP 回 error → 经 [`map_transport_err`] 成 `Other`（调用方判 Fatal）；节点 detach → CDP error。
    /// 在元素所属 `session` 上发（同进程 iframe 与父帧同 page session）。
    pub(crate) async fn set_file_input_files(
        &self,
        session: &str,
        object_id: &str,
        files: &[String],
    ) -> Result<(), BrowserError> {
        use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
        let params = SetFileInputFilesParams {
            files: files.to_vec(),
            node_id: None,
            backend_node_id: None,
            object_id: Some(RemoteObjectId::new(object_id.to_string())),
        };
        self.conn
            .send::<SetFileInputFilesParams>(session, &params)
            .await
            .map_err(map_transport_err)?;
        Ok(())
    }

    /// **[运行时] 读 file input 的 files 摘要**作 verify 锚点（upload_file）：`{count, first}`
    /// （files.length + 首文件名）。non-file 元素 / 异常 → None（best-effort）。**只读**。
    pub(crate) async fn act_read_file_input(
        &self,
        object_id: &str,
    ) -> Option<serde_json::Value> {
        let read_fn = "function() { \
             try { \
                 if (!this || this.tagName !== 'INPUT' || this.type !== 'file') return null; \
                 var f = this.files; \
                 var n = f ? f.length : 0; \
                 var first = (f && f.length > 0) ? f[0].name : null; \
                 return { count: n, first: first }; \
             } catch (e) { return null; } \
         }";
        let manager = self.injection_manager().await.ok()?;
        let result = manager.call_on_element(object_id, read_fn, true).await.ok()?;
        match result.get("value") {
            Some(v) if v.is_object() => Some(v.clone()),
            _ => None,
        }
    }

    /// **[运行时] 注入隐藏 `<a href=url download>` 并 click 触发下载**（download 选项 A）。在当前作用帧
    /// 的**默认 page world**（[`Self::active_frame_eval`]）跑——`a.click()` 在页面 world 当用户手势触发
    /// 下载（走 `Browser.setDownloadBehavior(allowAndName)` 沙箱 + downloadWillBegin/Progress 事件循环，
    /// E4 denylist/MOTW 全链生效）。url 经 JSON.stringify 安全内联。**不扰当前页**（创建游离 `<a>`，click
    /// 后立即移除）。异常 → 上抛（Fatal）。
    pub(crate) async fn trigger_anchor_download(&self, url: &str) -> Result<(), BrowserError> {
        let safe_url = serde_json::Value::String(url.to_string()).to_string();
        let expression = format!(
            "(() => {{ try {{ \
               const a = document.createElement('a'); \
               a.href = {safe_url}; \
               a.download = ''; \
               a.style.display = 'none'; \
               a.rel = 'noopener'; \
               document.body.appendChild(a); \
               a.click(); \
               a.remove(); \
               return true; \
             }} catch (e) {{ return false; }} }})()"
        );
        let value = self.active_frame_eval(&expression).await?;
        if value.as_bool().unwrap_or(false) {
            Ok(())
        } else {
            Err(BrowserError::Other(
                "failed to inject the download trigger (page may block dynamic anchors)".into(),
            ))
        }
    }

    /// **[运行时] 轮询隔离 downloads 目录至出现新增已完成文件**（download verify）。`before` 是触发前的
    /// 文件名集；每 [`DOWNLOAD_POLL_INTERVAL`] 扫一次目录，找**不在 `before`、size>0、且非 chrome 中间态**
    /// （`.crdownload`/`.tmp`）的文件即视作落盘完成，返 `(name, size)`。短 deadline
    /// （[`DOWNLOAD_SETTLE_TIMEOUT`]）内无新增 → `None`（良性，调用方报 success=false）。
    pub(crate) async fn poll_download_landed(
        &self,
        dir: &str,
        before: &std::collections::HashSet<String>,
    ) -> Result<Option<(String, u64)>, BrowserError> {
        let deadline = tokio::time::Instant::now() + DOWNLOAD_SETTLE_TIMEOUT;
        loop {
            if let Some(found) = newest_completed_download(dir, before)? {
                return Ok(Some(found));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(DOWNLOAD_POLL_INTERVAL).await;
        }
    }

    /// **[运行时] Page.printToPDF → 原始 PDF 字节**（save_as_pdf）。返回 base64-decode 后的 PDF bytes。
    /// 在 active tab 的 page session 上发（整页打印，非某帧）。headful 已实测可用（Chrome 149）；
    /// 某版本受限 / CDP 失败 → `Err`（绝不 panic）。
    pub(crate) async fn print_to_pdf(&self) -> Result<Vec<u8>, BrowserError> {
        let session = self.active_tab_handles().await?.session_id;
        // 默认参数：print_background=true（保留页面背景，更接近所见），其余默认（A4、不分页范围）。
        let params = PrintToPdfParams::builder().print_background(true).build();
        let result = self
            .conn
            .send::<PrintToPdfParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        let pdf: PrintToPdfReturns = serde_json::from_value(result.clone())
            .map_err(|e| BrowserError::Other(format!("parse printToPDF response: {e}")))?;
        // `data` 是 base64（同 captureScreenshot）。
        let b64: &str = pdf.data.as_ref();
        decode_base64(b64).ok_or_else(|| BrowserError::Other("printToPDF returned non-base64 data".into()))
    }

}
pub struct ActAbortGuard {
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for ActAbortGuard {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            // 监听任务只读事件 + 调 abort，无须优雅关闭：直接 abort 取消（动作已结束，事件不再相关）。
            h.abort();
        }
    }
}

/// **[纯逻辑] 事件 params 的 `sessionId` 是否匹配目标 session**（detach 接线判定）。
/// `Target.detachedFromTarget` 的 params 带 `sessionId`；与本 page session 比对，命中即
/// 「本动作所在 page 没了」。抽纯函数便于单测形状解析。
fn event_session_matches(params: &serde_json::Value, target_session: &str) -> bool {
    params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s == target_session)
        .unwrap_or(false)
}

/// `Target.targetCrashed` is emitted on the root session and identifies the
/// renderer by `targetId` (not `sessionId`).
fn event_target_matches(params: &serde_json::Value, target_id: &str) -> bool {
    params
        .get("targetId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|event_target| event_target == target_id)
}

/// **[纯逻辑] 事件 params 的 `frameId` 是否匹配目标帧**（frame detach 接线判定）。
/// `Page.frameDetached` 的 params 带 `frameId`（被 detach 的帧）；与动作所在帧比对，命中即「本动作
/// 所在帧从树上 detach」。抽纯函数便于单测形状解析。
fn event_frame_matches(params: &serde_json::Value, target_frame: &str) -> bool {
    params
        .get("frameId")
        .and_then(|v| v.as_str())
        .map(|s| s == target_frame)
        .unwrap_or(false)
}

/// **navigate settle 的产物**（[`CdpBackend::run_settle`] 返回）：达到的 settle 状态 + 是否走了
/// SPA 软导航降级路径（软导航是良性态，不升级 networkidle）。
struct SettleOutcome {
    state: NavSettleState,
    soft_nav: bool,
}

/// settle 各 select 分支收 broadcast 事件的「是否拿到一条有效事件」判定（D2）。`Ok(_)` → true；
/// `Lagged`（订阅落后）→ true（当作收到一次，继续推进——宁可早一步也不卡死；具体语义由各 absorb
/// 函数对 params 解析兜底）；`Closed`（连接没了）→ false（让外层 select 转向超时/其它分支兜底，
/// 绝不 busy-loop）。这里把 `Lagged` 当 true 是因为生命周期里程碑（DCL/load）即便丢了具体那条，
/// 后续仍能靠超时阶梯 + history 查 url 兜底，不致命。
fn recv_ok(
    ev: Result<crate::transport::CdpEvent, tokio::sync::broadcast::error::RecvError>,
) -> bool {
    use tokio::sync::broadcast::error::RecvError;
    match ev {
        Ok(_) => true,
        Err(RecvError::Lagged(_)) => true,
        Err(RecvError::Closed) => false,
    }
}

/// 吸收一条 `Network.responseReceived`：若是主帧 Document 响应，填 `http_status`（首个命中为准——
/// 主文档响应只有一个；后续子资源/子帧响应被 [`nav::extract_main_doc_status`] 过滤掉）。
fn absorb_response(
    ev: Result<crate::transport::CdpEvent, tokio::sync::broadcast::error::RecvError>,
    main_frame_id: &str,
    http_status: &mut Option<u16>,
) {
    if let Ok(ev) = ev
        && http_status.is_none()
        && let Some(s) = nav::extract_main_doc_status(&ev.params, main_frame_id)
    {
        *http_status = Some(s);
    }
}

/// 吸收一条 `Network.requestWillBeSent`：按是否「重定向续发」（有 redirectResponse）+1 / 不变。
fn absorb_request(
    ev: Result<crate::transport::CdpEvent, tokio::sync::broadcast::error::RecvError>,
    inflight: &mut InflightCounter,
) {
    if let Ok(ev) = ev {
        inflight.on_request_will_be_sent(nav::request_is_redirect(&ev.params));
    }
}

/// 吸收一条 `Network.loadingFinished`：-1（钳零）。
fn absorb_finish(
    ev: Result<crate::transport::CdpEvent, tokio::sync::broadcast::error::RecvError>,
    inflight: &mut InflightCounter,
) {
    if ev.is_ok() {
        inflight.on_loading_finished();
    }
}

/// 吸收一条 `Network.loadingFailed`：-1（钳零）。
fn absorb_fail(
    ev: Result<crate::transport::CdpEvent, tokio::sync::broadcast::error::RecvError>,
    inflight: &mut InflightCounter,
) {
    if ev.is_ok() {
        inflight.on_loading_failed();
    }
}

/// 一个已观测帧：seq（拼 `f<seq>` 前缀）+ frame_id + 所属 session + 单帧快照。
struct ObservedFrame {
    seq: u32,
    frame_id: String,
    session_id: String,
    snapshot: FrameSnapshot,
}

/// D5：收集单帧 password 输入的 aria ref（同帧 utility world），追加进 `out`。
///
/// **fail-closed 契约**：返回 `true` 表示该帧 password 探测**失败**（`password_refs` 返 `Err`）。
/// 失败时无法精确知道哪些字段是 password，**不得**只 warn 放行——调用方据此对全部可编辑控件值
/// over-redact 兜底（见 [`CdpBackend::observe_impl`] step 6）。正常路径（`Ok`）返回 `false` 并把
/// 该帧 password ref 追加进 `out`，缝合后宿主侧精确抹其 value。
async fn collect_password_refs(
    manager: &InjectionManager,
    frame_id: &str,
    out: &mut Vec<String>,
) -> bool {
    match manager.password_refs(frame_id).await {
        Ok(refs) => {
            out.extend(refs);
            false
        }
        Err(e) => {
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                frame_id = %frame_id, error = ?e,
                "collect password refs failed (D5: will fail-closed over-redact all editable values)"
            );
            true
        }
    }
}

/// Append to one bounded output buffer.  Capacity is checked before every
/// write, so indentation amplification cannot allocate past the task limit.
fn push_observation_bytes(
    out: &mut String,
    value: &str,
) -> Result<(), ObservationCapacityError> {
    let attempted = out.len().saturating_add(value.len());
    ensure_observation_bytes(attempted)?;
    out.push_str(value);
    Ok(())
}

fn push_observation_spaces(
    out: &mut String,
    count: usize,
) -> Result<(), ObservationCapacityError> {
    let attempted = out.len().saturating_add(count);
    ensure_observation_bytes(attempted)?;
    const SPACES: &str = "                                                                ";
    let mut remaining = count;
    while remaining != 0 {
        let chunk = remaining.min(SPACES.len());
        out.push_str(&SPACES[..chunk]);
        remaining -= chunk;
    }
    Ok(())
}

fn iframe_ref_slice(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("- iframe") && !trimmed.contains("iframe ") {
        return None;
    }
    let start = line.find("[ref=")? + 5;
    let end = line[start..].find(']')? + start;
    Some(&line[start..end])
}

/// Render an iframe tree directly into one bounded String.  The former
/// implementation recursively materialized every child subtree and cloned it
/// again into every ancestor, producing O(depth * payload) retained copies on
/// deep frame trees.  This emitter visits each rendered line once and never
/// owns an intermediate subtree String.
fn render_frame_recursive_bounded(
    frames: &[ObservedFrame],
    idx: usize,
    parent_of: &HashMap<String, (String, String)>,
) -> Result<String, ObservationCapacityError> {
    let mut children: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    for (cidx, cf) in frames.iter().enumerate() {
        if let Some((pfid, iref)) = parent_of.get(&cf.frame_id)
        {
            children
                .entry(pfid.as_str())
                .or_default()
                .insert(iref.as_str(), cidx);
        }
    }

    fn render_into(
        frames: &[ObservedFrame],
        idx: usize,
        children: &HashMap<&str, HashMap<&str, usize>>,
        base_indent: usize,
        visiting: &mut HashSet<usize>,
        out: &mut String,
        first_line: &mut bool,
    ) -> Result<(), ObservationCapacityError> {
        if !visiting.insert(idx) {
            return Err(ObservationCapacityError::new(
                MAX_OBSERVATION_RETAINED_BYTES.saturating_add(1),
            ));
        }
        let frame = &frames[idx];
        for line in frame.snapshot.full.lines() {
            if !*first_line {
                push_observation_bytes(out, "\n")?;
            }
            *first_line = false;
            push_observation_spaces(out, base_indent)?;

            let child = iframe_ref_slice(line).and_then(|reference| {
                children
                    .get(frame.frame_id.as_str())
                    .and_then(|by_ref| by_ref.get(reference))
                    .copied()
                    .map(|child_idx| (reference, child_idx))
            });
            if let Some((reference, child_idx)) = child {
                let head = line.trim_end();
                push_observation_bytes(out, head)?;
                if !head.ends_with(':') {
                    push_observation_bytes(out, ":")?;
                }
                let depth = frame
                    .snapshot
                    .iframe_depths
                    .get(reference)
                    .copied()
                    .unwrap_or(0);
                let relative_indent = usize::try_from(depth)
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    .and_then(|value| value.checked_mul(2))
                    .ok_or_else(|| {
                        ObservationCapacityError::new(
                            MAX_OBSERVATION_RETAINED_BYTES.saturating_add(1),
                        )
                    })?;
                let child_indent = base_indent.checked_add(relative_indent).ok_or_else(|| {
                    ObservationCapacityError::new(
                        MAX_OBSERVATION_RETAINED_BYTES.saturating_add(1),
                    )
                })?;
                ensure_observation_bytes(child_indent)?;
                render_into(
                    frames,
                    child_idx,
                    children,
                    child_indent,
                    visiting,
                    out,
                    first_line,
                )?;
            } else {
                push_observation_bytes(out, line)?;
            }
        }
        visiting.remove(&idx);
        Ok(())
    }

    let mut out = String::new();
    let mut visiting = HashSet::new();
    let mut first_line = true;
    render_into(
        frames,
        idx,
        &children,
        0,
        &mut visiting,
        &mut out,
        &mut first_line,
    )?;
    Ok(out)
}

/// 单帧是否触及 depth 封顶（粗判：full 里出现缩进达 `(max_depth)*2` 空格的行——
/// renderAriaTree 在 depth==limit 时仍渲染该层但不再下钻，故粗判用于 truncated 标志）。
fn frame_hit_depth_limit(snap: &FrameSnapshot, max_depth: u32) -> bool {
    if max_depth == 0 {
        return false;
    }
    let limit_indent = (max_depth as usize) * 2;
    snap.full.lines().any(|l| {
        let lead = l.len() - l.trim_start().len();
        lead >= limit_indent
    })
}

/// 从 aria 行抽 `[ref=...]` 的 ref 值（不含 `[cursor=pointer]` 等后缀）。
fn parse_ref_token(line: &str) -> Option<String> {
    let start = line.find("[ref=")? + 5;
    let end = line[start..].find(']')? + start;
    Some(line[start..end].to_string())
}

/// 从 `f<seq>e<n>` 形态的 ref 抽 `<seq>`。形态不符返回 None。
fn parse_seq_from_ref(reff: &str) -> Option<u32> {
    let rest = reff.strip_prefix('f')?;
    let e_pos = rest.find('e')?;
    rest[..e_pos].parse::<u32>().ok()
}

/// 从 aria 行抽 (role, name)。行形如 `  - button "Submit order" [ref=f0e1]` 或 `- iframe [ref=f0e5]`。
/// role = `- ` 后到首个空格/引号/`[` 的 token；name = 首个 `"..."`（无引号串则空）。
fn parse_role_name(line: &str) -> (String, String) {
    let t = line.trim_start();
    let t = t.strip_prefix("- ").unwrap_or(t);
    // role：到首个 ' ' / '"' / '[' 为止。
    let role_end = t.find([' ', '"', '[']).unwrap_or(t.len());
    let role = t[..role_end].trim().to_string();
    // name：首个双引号包裹的串（aria 用 JSON.stringify，故内部转义按 JSON；这里取裸内容到下个未转义引号）。
    let name = extract_quoted(t).unwrap_or_default();
    (role, name)
}

/// 抽行内首个双引号包裹的串内容（尊重 `\"` 转义）。无引号串返回 None。
fn extract_quoted(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let start = s.find('"')? + 1;
    let mut out = String::new();
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if c == '"' {
            return Some(out);
        }
        out.push(c);
        i += 1;
    }
    None
}

/// 标准 base64 解码（CDP 截图 `data` 是干净的标准 base64）。用 workspace 的 `base64`
/// crate（与全仓惯例一致，免手写维护）。
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

// ═══════════════════════════════════════════════════════════════════════════
// F-actions：download 落盘探测 + save_as_pdf 输出路径（纯逻辑 free 函数，便于单测）。
// ═══════════════════════════════════════════════════════════════════════════

/// download verify 的落盘探测短超时（触发后等隔离 downloads 目录出现新增已完成文件的上限）。
/// 比 action 默认略宽（下载经网络 + 落盘异步），但远小于 nav 30s（避免整轮挂死）；超时即 success=false
/// 如实（良性：可能被红线取消 / url 无附件 / 仍在传）。
const DOWNLOAD_SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// download 落盘探测的轮询间隔（每隔这么久扫一次目录）。
const DOWNLOAD_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Before/after download verification snapshots must not scale with an
/// attacker-populated workspace directory. Entry count bounds filesystem work;
/// the separate byte bound covers many long names below that count.
const MAX_DOWNLOAD_DIRECTORY_SNAPSHOT_ENTRIES: usize = 4_096;
const MAX_DOWNLOAD_DIRECTORY_NAME_BYTES: usize = 256 * 1024;
const MAX_DOWNLOAD_DIRECTORY_SINGLE_NAME_BYTES: usize = 1_024;

/// **[纯逻辑] 列一个目录下的「文件名」集合**（download 落盘探测的触发前基线）。目录不存在 / 读不了 →
/// 空集（best-effort，绝不 panic）。只收**文件**（非子目录）的文件名（`file_name()` 的 lossy 串）。
/// `pub(crate)`：actions.rs 的 act_download 取触发前基线用。
pub(crate) fn list_dir_files(
    dir: &str,
) -> Result<std::collections::HashSet<String>, BrowserError> {
    let mut set = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(set);
    };
    let mut name_bytes = 0usize;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DOWNLOAD_DIRECTORY_SNAPSHOT_ENTRIES {
            return Err(download_directory_snapshot_capacity_error(
                "entry count",
            ));
        }
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        account_download_directory_name(&mut name_bytes, name.len())?;
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            set.insert(name.into_owned());
        }
    }
    Ok(set)
}

/// **[纯逻辑] 在 downloads 目录里找一个「新增的、已完成的」下载文件**（download verify 单步探测）。
/// 「新增」= 文件名不在 `before` 基线集；「已完成」= size>0 且**非 chrome 中间态**（`.crdownload`
/// / `.tmp` 后缀是下载进行中的临时文件，不算落盘完成）。命中返 `(name, size)`（首个满足的）；无 → None。
/// best-effort：读目录 / 取元数据失败 → 跳过该项（绝不 panic）。
fn newest_completed_download(
    dir: &str,
    before: &std::collections::HashSet<String>,
) -> Result<Option<(String, u64)>, BrowserError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(None);
    };
    let mut name_bytes = 0usize;
    let mut found = None;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DOWNLOAD_DIRECTORY_SNAPSHOT_ENTRIES {
            return Err(download_directory_snapshot_capacity_error(
                "entry count",
            ));
        }
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        account_download_directory_name(&mut name_bytes, name.len())?;
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = name.into_owned();
        // 已存在（触发前就在）→ 非本次下载，跳过。
        if before.contains(&name) {
            continue;
        }
        // chrome 下载中间态（仍在传）→ 不算完成，跳过。
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".crdownload") || lower.ends_with(".tmp") {
            continue;
        }
        // size>0 才算落盘完成（0 字节多是刚创建占位）。
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size > 0 && found.is_none() {
            found = Some((name, size));
        }
    }
    Ok(found)
}

fn account_download_directory_name(
    total_name_bytes: &mut usize,
    name_bytes: usize,
) -> Result<(), BrowserError> {
    if name_bytes > MAX_DOWNLOAD_DIRECTORY_SINGLE_NAME_BYTES {
        return Err(download_directory_snapshot_capacity_error(
            "single filename bytes",
        ));
    }
    *total_name_bytes = total_name_bytes.saturating_add(name_bytes);
    if *total_name_bytes > MAX_DOWNLOAD_DIRECTORY_NAME_BYTES {
        return Err(download_directory_snapshot_capacity_error(
            "total filename bytes",
        ));
    }
    Ok(())
}

fn download_directory_snapshot_capacity_error(limit: &str) -> BrowserError {
    BrowserError::Blocked {
        reason: format!(
            "sandboxed download directory exceeds its bounded {limit}; clean the task workspace before retrying"
        ),
    }
}

/// **[纯逻辑] save_as_pdf 的输出文件路径**：`<downloads_dir>/page-<unix_ts_ms>.pdf`。时间戳（毫秒）
/// 防同会话多次 save_as_pdf 覆盖。系统时钟异常（早于 UNIX 纪元，几乎不可能）→ 退 `page-0.pdf`。
/// `pub(crate)`：actions.rs 的 act_save_as_pdf 算落点用。
pub(crate) fn pdf_output_path(downloads_dir: &str) -> std::path::PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::path::Path::new(downloads_dir).join(format!("page-{ts}.pdf"))
}

/// **E4 下载沙箱接线**：在**根 browser session** 挂 `Browser.setDownloadBehavior`。
///
/// `behavior = allowAndName`（让 chrome 用下载 GUID 命名落盘文件，规避同名覆盖/路径穿越攻击），
/// `downloadPath = <per-pet workspace>/downloads`（隔离落点，**绝不**用户真实 Downloads），
/// `eventsEnabled = true`（开 `downloadWillBegin`/`downloadProgress` 事件——MOTW 循环靠它知道
/// 落盘完成的 `filePath`）。
///
/// 浏览器级（ROOT_SESSION）：作用于默认 browser context 的所有 page，故对当前及后开标签页统一生效。
/// `browser_context_id: None` = 默认 context。
async fn set_download_behavior_sandbox(
    conn: &Connection,
    download_path: &str,
) -> Result<(), BrowserError> {
    let params = SetDownloadBehaviorParams {
        behavior: SetDownloadBehaviorBehavior::AllowAndName,
        browser_context_id: None,
        download_path: Some(download_path.to_string()),
        events_enabled: Some(true),
    };
    conn.send::<SetDownloadBehaviorParams>(ROOT_SESSION, &params)
        .await
        .map_err(map_transport_err)?;
    Ok(())
}

/// Fail-closed download posture for a Host without an exclusive staging
/// identity: Chromium must never write into its default (user Downloads)
/// directory on our behalf.
async fn set_download_behavior_deny(conn: &Connection) -> Result<(), BrowserError> {
    let params = SetDownloadBehaviorParams {
        behavior: SetDownloadBehaviorBehavior::Deny,
        browser_context_id: None,
        download_path: None,
        events_enabled: Some(false),
    };
    conn.send::<SetDownloadBehaviorParams>(ROOT_SESSION, &params)
        .await
        .map_err(map_transport_err)?;
    Ok(())
}

/// **E4 下载事件后台循环**：订阅 `Browser.downloadProgress`，对**完成**的下载在其落盘文件上打
/// Win MOTW（`Zone.Identifier` ADS）。
///
/// `downloadProgress` 的最后一次调用 `state=="completed"` 且（在桌面平台）`filePath` 给出落盘的
/// 实际路径（`allowAndName` 下是 `downloads/<GUID>`）。我们对该文件调
/// [`crate::download::write_motw`]——Windows 真写 ADS，mac/linux 空实现。**绝不**自动打开/启动文件。
///
/// best-effort：MOTW 是纵深防御附加层，写失败（非 NTFS / 文件已被移走）只 `debug` 不致命。连接关闭
/// （`RecvError::Closed`）→ 退出循环（backend Drop 关连接即触发）。
///
/// **E4 下载事件后台循环 + F1-sec 可执行下载红线 enforcement**。
///
/// 订阅两个事件：
/// 1. **`Browser.downloadWillBegin`**（F1-sec 接线点）：下载**发起**时即给出 `suggestedFilename`。
///    命中 [`crate::download::reject_executable_download`]（可执行/脚本 denylist）→ 立刻
///    `Browser.cancelDownload{guid}` **取消**该下载（fail-closed，**红线**——yolo/companion 也取消，
///    因为这道判定**不看 session_mode**：denylist 命中即拒，无放行参数，见 `reject_executable_download`
///    的红线语义）。这正是「可执行下载在红线会话也拒」的真实 enforcement（在落盘之前拦下）。
///    **F5：本红线事件走 `subscribe_reliable`（无损）**——lossy broadcast 在事件突发（>容量）时
///    静默丢事件，等于用时序就能绕过红线；控制事件绝不能丢。
/// 2. **`Browser.downloadProgress`**：对**完成**（`state=="completed"`）的下载在其落盘文件打 Win MOTW
///    （`Zone.Identifier` ADS，Windows 真写 / mac-linux 空实现）。**绝不**自动打开/启动文件。
///    仍走 lossy broadcast（MOTW/落盘路由是 best-effort 纵深层，非红线控制事件）。
///
/// 二者互补：`downloadWillBegin` 在**发起**侧拦可执行（早于落盘）；`downloadProgress` 在**完成**侧对
/// 放行的非可执行下载打 MOTW。被取消的可执行下载不会走到 completed，故不打 MOTW（也无需）。
///
/// best-effort + 绝不 panic：解析失败 / cancel 失败 / MOTW 写失败只 `warn`/`debug`，不致命。连接关闭
/// （可靠通道 `None` / `RecvError::Closed`）→ 退出循环（backend Drop 关连接即触发）。
fn spawn_download_loop(
    conn: Connection,
    router: Option<Arc<HostTargetRouter>>,
) -> tokio::task::JoinHandle<()> {
    let mut begin_rx = conn.subscribe_reliable(EventDownloadWillBegin::IDENTIFIER, None);
    let mut progress_rx = conn.subscribe(EventDownloadProgress::IDENTIFIER, None);
    tokio::spawn(async move {
        let mut reconcile_tick = tokio::time::interval(DOWNLOAD_RECONCILE_INTERVAL);
        reconcile_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                // ① 下载发起 → 可执行 denylist 红线（命中即 cancelDownload，fail-closed/yolo 也取消）。
                ev = begin_rx.recv() => {
                    match ev {
                        Some(ev) => {
                            let b = match serde_json::from_value::<EventDownloadWillBegin>(ev.params.clone()) {
                                Ok(begin) => begin,
                                Err(error) => {
                                    if let Some(guid) = ev.params.get("guid").and_then(serde_json::Value::as_str) {
                                        if let Some(router) = &router {
                                            if !router.quarantine_rejected_download(guid) {
                                                conn.shutdown().await;
                                                break;
                                            }
                                        }
                                        cancel_download_best_effort(
                                            &conn,
                                            guid,
                                            "unparseable download begin event",
                                        )
                                        .await;
                                    }
                                    tracing::warn!(%error, "unparseable download begin event denied");
                                    continue;
                                }
                            };
                            // SD-3: Two complementary checks — filename extension denylist OR
                            // data: URL content sniffing. Either triggers the red-line cancel.
                            let filename_blocked = crate::download::reject_executable_download(&b.suggested_filename).is_err();
                            let content_blocked = crate::download::data_url_is_executable(&b.url);

                            if filename_blocked || content_blocked {
                                let reason = if content_blocked && !filename_blocked {
                                    "data: URL content sniffed as executable (magic bytes match)"
                                } else if filename_blocked && content_blocked {
                                    "executable filename extension AND data: URL content sniffed as executable"
                                } else {
                                    "executable/script filename extension"
                                };
                                tracing::warn!(
                                    target: "nomi_browser_engine::backend::cdp",
                                    guid = %b.guid, suggested = %b.suggested_filename,
                                    url_scheme = %blocked_download_url_scheme(&b.url),
                                    reason = %reason,
                                    "download blocked (red-line, denied even under yolo/companion); cancelling"
                                );
                                if let Some(router) = &router {
                                    if !router.quarantine_rejected_download(&b.guid) {
                                        conn.shutdown().await;
                                        break;
                                    }
                                }
                                cancel_download_best_effort(&conn, &b.guid, "blocked red-line download").await;
                            } else if let Some(router) = &router {
                                let admitted = router
                                    .begin_download(
                                        b.frame_id.as_ref(),
                                        &b.guid,
                                        &b.suggested_filename,
                                    )
                                    .await;
                                if !admitted {
                                    if !router.quarantine_rejected_download(&b.guid) {
                                        conn.shutdown().await;
                                        break;
                                    }
                                    cancel_download_best_effort(
                                        &conn,
                                        &b.guid,
                                        "unowned, duplicate, or over-capacity download",
                                    )
                                    .await;
                                }
                            }
                        }
                        None => {
                            if let Some(router) = &router {
                                router.poison_downloads_for_host_stop();
                            }
                            break;
                        }
                    }
                }
                // ② 下载完成 → 对放行的非可执行下载打 MOTW（被取消的可执行不会到这里）。
                ev = progress_rx.recv() => {
                    match ev {
                        Ok(ev) => {
                            let Ok(p) = serde_json::from_value::<EventDownloadProgress>(ev.params.clone())
                            else {
                                if let Some(router) = &router {
                                    for guid in router.poison_downloads_for_host_stop() {
                                        cancel_download_best_effort(
                                            &conn,
                                            &guid,
                                            "unparseable download progress",
                                        )
                                        .await;
                                    }
                                }
                                continue;
                            };
                            use chromiumoxide::cdp::browser_protocol::browser::DownloadProgressState;
                            if p.state == DownloadProgressState::Canceled {
                                if let Some(router) = &router {
                                    router.cancel_pending_download(&p.guid);
                                }
                                continue;
                            }
                            if let Some(router) = &router {
                                let received_bytes = download_progress_bytes(p.received_bytes);
                                let total_bytes = if p.total_bytes == 0.0 {
                                    Some(None)
                                } else {
                                    download_progress_bytes(p.total_bytes).map(Some)
                                };
                                let within_task_boundary = match (received_bytes, total_bytes) {
                                    (Some(received_bytes), Some(total_bytes)) => router
                                        .update_download_progress(
                                            &p.guid,
                                            received_bytes,
                                            total_bytes,
                                        ),
                                    _ => false,
                                };
                                if !within_task_boundary {
                                    if !router.quarantine_rejected_download(&p.guid) {
                                        conn.shutdown().await;
                                        break;
                                    }
                                    cancel_download_best_effort(
                                        &conn,
                                        &p.guid,
                                        "invalid or over-capacity download progress",
                                    )
                                    .await;
                                    continue;
                                }
                            }
                            if p.state == DownloadProgressState::InProgress {
                                continue;
                            }
                            // A completed event without filePath is still a
                            // terminal state.  Forget its route and remove the
                            // deterministic GUID staging paths.
                            let Some(file_path) = p.file_path.as_deref() else {
                                if let Some(router) = &router {
                                    router.cancel_pending_download(&p.guid);
                                }
                                tracing::debug!(guid = %p.guid, "download completed without filePath; route reconciled");
                                continue;
                            };
                            let path = std::path::Path::new(file_path);
                            if let Some(router) = &router {
                                if router.download_cancel_requested(&p.guid) {
                                    router.cancel_pending_download(&p.guid);
                                    router.cleanup_staged_download(&p.guid, Some(path));
                                    continue;
                                }
                                if router.finish_download(&p.guid, path).await {
                                    continue;
                                }
                                // Unknown/blocked/failed routes must never be
                                // promoted out of Host staging.  Exact staging
                                // children are deleted; arbitrary paths from a
                                // malformed event are left untouched.
                                router.cleanup_staged_download(&p.guid, Some(path));
                                if router
                                    .download_ledger
                                    .download_cleanup_poisoned
                                    .load(Ordering::Acquire)
                                {
                                    router.poison_downloads_for_host_stop();
                                    conn.shutdown().await;
                                    break;
                                }
                                continue;
                            }
                            match crate::download::write_motw(path) {
                                Ok(()) => {
                                    tracing::debug!(file = %file_path, "download completed; MOTW applied (Windows)");
                                }
                                Err(e) => {
                                    tracing::debug!(error = %e, file = %file_path, "MOTW write failed (non-NTFS or file moved); benign");
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            if let Some(router) = &router {
                                router.poison_downloads_for_host_stop();
                            }
                            break;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            if let Some(router) = &router {
                                let guids = router.poison_downloads_for_host_stop();
                                tracing::warn!(
                                    skipped,
                                    reconciled = guids.len(),
                                    "download progress stream lagged; cancelling all tracked downloads fail-closed"
                                );
                                for guid in guids {
                                    cancel_download_best_effort(
                                        &conn,
                                        &guid,
                                        "lagged download progress stream",
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
                _ = reconcile_tick.tick() => {
                    if let Some(router) = &router {
                        for guid in router.expire_pending_downloads() {
                            cancel_download_best_effort(
                                &conn,
                                &guid,
                                "download routing TTL expired",
                            )
                            .await;
                        }
                        router.sweep_stale_staging_files();
                        router.retry_staging_cleanup();
                        if router.cancel_terminal_grace_expired()
                            || router
                                .download_ledger
                                .download_cleanup_poisoned
                                .load(Ordering::Acquire)
                        {
                            router.poison_downloads_for_host_stop();
                            conn.shutdown().await;
                            break;
                        }
                    }
                }
            }
        }
        if let Some(router) = &router {
            router.poison_downloads_for_host_stop();
        }
    })
}

fn download_progress_bytes(value: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        return None;
    }
    Some(value.ceil() as u64)
}

async fn cancel_download_best_effort(
    conn: &Connection,
    guid: &str,
    reason: &'static str,
) -> bool {
    if let Err(error) = conn
        .send_may_fail::<CancelDownloadParams>(
            ROOT_SESSION,
            &CancelDownloadParams::new(guid.to_string()),
        )
        .await
    {
        tracing::warn!(
            target: "nomi_browser_engine::backend::cdp",
            %guid,
            %error,
            reason,
            "Browser.cancelDownload failed; terminal/staging reconciliation remains armed"
        );
        conn.shutdown().await;
        false
    } else {
        true
    }
}

/// 取 URL 的 scheme（含冒号，如 `"https:"`）供**阻断日志**使用。**绝不 panic**（F46）：
/// 旧实现 `&url[..url.find(':').unwrap_or(0).min(10) + 1]` 对空 URL 越界、对冒号前含
/// 多字节字符的非常规 URL 可能切在 char 边界内——panic 会杀死整条下载红线循环，之后
/// 可执行下载不再被取消。空串/无冒号 → `"[no-scheme]"`；冒号位置 >10 字节（非常规
/// scheme）→ 不切片，返回 `"[odd-scheme]"`。
fn blocked_download_url_scheme(url: &str) -> &str {
    if url.starts_with("data:") {
        return "data:";
    }
    match url.find(':') {
        // ':' 是单字节 ASCII，`..=pos` 的右边界紧跟其后，恒为合法 char 边界。
        Some(pos) if pos <= 10 => &url[..=pos],
        Some(_) => "[odd-scheme]",
        None => "[no-scheme]",
    }
}

/// **E5 出口防火墙：对单个 session 挂 `Fetch.enable`**（全流量拦截）。
///
/// `EnableParams::default()`（空 `patterns`）= 拦截**所有** Request 阶段的请求（nav + XHR + fetch +
/// POST + 子资源 + beacon）。对根 browser / page / OOPIF / **service_worker** session 都调本函数——
/// **SW 必须也拦**（裁决⑪/不变量⑬：否则页面把出口请求塞进 SW 即整体绕过防火墙）。
///
/// 用 `send_may_fail`：session 可能在挂之前就 detach（target 关闭竞态），吞掉「目标已不在」类错误
/// （防火墙对一个已消失的 session 失效本就无害）。
async fn enable_fetch_on_session(conn: &Connection, session_id: &str) -> Result<(), TransportError> {
    let params = FetchEnableParams::default();
    conn.send_may_fail::<FetchEnableParams>(session_id, &params)
        .await
}

/// **F1：防火墙循环的可靠订阅，必须在 attach loop 启动之前注册。**
///
/// `Connection::handle_attached` 只在存在 `Fetch.requestPaused` 可靠订阅者时才对
/// attach 的 session 挂 `Fetch.enable`（无消费者的 requestPaused 事件被静默丢弃且
/// CDP 不重发——请求会永久卡死）。构造器先建本订阅（底层 receiver 保持可靠语义，同时由 transport
/// 的条数/字节额度做逻辑硬边界，事件在循环 spawn 前缓存不丢）、再 `run_attach_loop()`、最后把它交给
/// [`spawn_fetch_firewall_loop`]，
/// 保证「先有消费者、后开拦截」在结构上恒成立。
struct FetchFirewallSubscriptions {
    attached_rx: tokio::sync::mpsc::UnboundedReceiver<crate::transport::CdpEvent>,
    paused_rx: tokio::sync::mpsc::UnboundedReceiver<crate::transport::CdpEvent>,
}

#[derive(Clone, Copy)]
struct FirewallExecutorLimits {
    request_workers: usize,
    request_queue_capacity: usize,
    approval_workers: usize,
    approval_queue_capacity: usize,
    approval_timeout: Duration,
    shutdown_join_timeout: Duration,
}

impl Default for FirewallExecutorLimits {
    fn default() -> Self {
        Self {
            request_workers: FIREWALL_REQUEST_WORKERS,
            request_queue_capacity: FIREWALL_REQUEST_QUEUE_CAPACITY,
            approval_workers: FIREWALL_APPROVAL_WORKERS,
            approval_queue_capacity: FIREWALL_APPROVAL_QUEUE_CAPACITY,
            approval_timeout: crate::firewall::EGRESS_APPROVAL_TIMEOUT,
            shutdown_join_timeout: FIREWALL_SHUTDOWN_JOIN_TIMEOUT,
        }
    }
}

enum FirewallRequestJob {
    EnableSession(crate::transport::CdpEvent),
    Paused(crate::transport::CdpEvent),
}

struct FirewallApprovalJob {
    conn: Connection,
    session_id: String,
    request_id: chromiumoxide::cdp::browser_protocol::fetch::RequestId,
    /// Normalized host only; never retain the potentially very large request
    /// URL/body in the slow approval queue.
    target: String,
    preview: crate::firewall::PostPreview,
    approver: Arc<dyn crate::firewall::EgressApprover>,
    approved_domains: crate::firewall::ApprovedDomains,
}

/// Host-owned firewall task tree. The watchdog owns the loop `JoinHandle`,
/// while this value retains exact cancellation/abort authority and joins the
/// whole registered worker tree during explicit shutdown.
struct FirewallLoopRuntime {
    cancel: CancellationToken,
    loop_abort: tokio::task::AbortHandle,
    watchdog: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown_join_timeout: Duration,
}

impl FirewallLoopRuntime {
    fn take_watchdog(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.watchdog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    async fn shutdown(&self) {
        self.cancel.cancel();
        let Some(mut watchdog) = self.take_watchdog() else {
            return;
        };
        if tokio::time::timeout(self.shutdown_join_timeout, &mut watchdog)
            .await
            .is_err()
        {
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                "firewall worker tree exceeded its shutdown join budget; aborting the bounded task tree"
            );
            self.loop_abort.abort();
            watchdog.abort();
            let _ = watchdog.await;
        }
    }

    fn abort(&self) {
        self.cancel.cancel();
        self.loop_abort.abort();
        if let Some(watchdog) = self.take_watchdog() {
            watchdog.abort();
        }
    }

    /// Convert a not-yet-published firewall tree into one bounded supervisor
    /// handle. The watchdog owns the loop JoinHandle and registered workers;
    /// the shared cleanup relay will retain and join this handle to terminal.
    fn into_pending_cleanup_handle(self) -> Option<tokio::task::JoinHandle<()>> {
        self.cancel.cancel();
        self.loop_abort.abort();
        self.take_watchdog()
    }
}

impl Drop for FirewallLoopRuntime {
    fn drop(&mut self) {
        self.abort();
    }
}

impl FetchFirewallSubscriptions {
    fn subscribe(conn: &Connection) -> Self {
        Self {
            attached_rx: conn.subscribe_reliable(EventAttachedToTarget::IDENTIFIER, None),
            paused_rx: conn.subscribe_reliable(EventRequestPaused::IDENTIFIER, None),
        }
    }
}

/// **E5 出口防火墙后台循环**：①消费 `Target.attachedToTarget`（全 session 通配）→ 对每个新 session
/// （page / OOPIF / **service_worker**）挂 `Fetch.enable`；②消费 `Fetch.requestPaused`（全 session
/// 通配）→ 对每条被拦请求经 [`crate::firewall::decide`] 判定 → 在**事件自身的 sessionId** 上发
/// `Fetch.continueRequest`（放行）/ `Fetch.failRequest{BlockedByClient}`（阻断）。
///
/// **订阅先于循环（F1）**：两路可靠订阅由调用方经 [`FetchFirewallSubscriptions::subscribe`]
/// 在 attach loop 启动**之前**注册后传入——`handle_attached` 的 Fetch.enable arming gate
/// 依赖该订阅已存在；订阅与循环 spawn 之间的事件由 transport 的可靠有界额度保留，循环启动后补处理。
///
/// **SW 链路（裁决⑪/不变量⑬）**：本循环消费的 `attachedToTarget` 含 service_worker（P0 保持其
/// attach、不 detach），故 SW session 同样被挂 `Fetch.enable`、其出口请求同样经本循环判定——SW 无法
/// 绕过防火墙。
///
/// **跨域 POST 门控的 enforcement 边界（E5 范围）**：[`FirewallDecision::GatePost`] 构造严格有界且
/// 不含字段值的预览；有审批器时交给 Host-owned 固定 worker 与有界队列，队列饱和或审批超时均
/// fail-closed，绝不为每条请求派生 detached task。无审批器的托管上下文维持既有“放行 + 审计”产品
/// 语义。[`FirewallDecision::Block`]（IP 封禁）是硬阻断，E5 即 enforce（SSRF 防护无审批语义）。
///
/// 所有错误 best-effort：单条请求判定/dispatch 失败只 `debug`/`warn`，**绝不 panic**，且**绝不**让一条
/// 请求悬挂（任何分支都对它 continue 或 fail——否则 Fetch.enable 下未应答的请求会卡住页面）。连接关闭
/// （可靠通道 `None`）→ 退出循环（backend Drop 关连接即触发）。
fn spawn_fetch_firewall_loop(
    conn: Connection,
    subscriptions: FetchFirewallSubscriptions,
    config: crate::firewall::FirewallConfig,
    egress_approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
    approved_domains: crate::firewall::ApprovedDomains,
    dns_resolver: Arc<dyn crate::firewall::HostResolver>,
    dns_cache: crate::firewall::DnsResolverCache,
) -> FirewallLoopRuntime {
    spawn_fetch_firewall_loop_with_limits(
        conn,
        subscriptions,
        config,
        egress_approver,
        approved_domains,
        dns_resolver,
        dns_cache,
        FirewallExecutorLimits::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_fetch_firewall_loop_with_limits(
    conn: Connection,
    subscriptions: FetchFirewallSubscriptions,
    config: crate::firewall::FirewallConfig,
    egress_approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
    approved_domains: crate::firewall::ApprovedDomains,
    dns_resolver: Arc<dyn crate::firewall::HostResolver>,
    dns_cache: crate::firewall::DnsResolverCache,
    limits: FirewallExecutorLimits,
) -> FirewallLoopRuntime {
    let cancel = CancellationToken::new();
    let loop_cancel = cancel.clone();
    let loop_conn = conn.clone();
    let firewall_loop = tokio::spawn(async move {
        run_fetch_firewall_loop(
            loop_conn,
            subscriptions,
            config,
            egress_approver,
            approved_domains,
            dns_resolver,
            dns_cache,
            limits,
            loop_cancel,
        )
        .await;
    });
    let loop_abort = firewall_loop.abort_handle();
    let watchdog = spawn_firewall_watchdog(conn, firewall_loop);
    FirewallLoopRuntime {
        cancel,
        loop_abort,
        watchdog: std::sync::Mutex::new(Some(watchdog)),
        shutdown_join_timeout: limits.shutdown_join_timeout,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_fetch_firewall_loop(
    conn: Connection,
    subscriptions: FetchFirewallSubscriptions,
    config: crate::firewall::FirewallConfig,
    egress_approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
    approved_domains: crate::firewall::ApprovedDomains,
    dns_resolver: Arc<dyn crate::firewall::HostResolver>,
    dns_cache: crate::firewall::DnsResolverCache,
    limits: FirewallExecutorLimits,
    cancel: CancellationToken,
) {
    let FetchFirewallSubscriptions {
        mut attached_rx,
        mut paused_rx,
    } = subscriptions;
    let (request_tx, request_rx) = tokio::sync::mpsc::channel(
        limits.request_queue_capacity.max(1),
    );
    let request_rx = Arc::new(AsyncMutex::new(request_rx));

    let approvals_enabled = egress_approver.is_some();
    let (approval_tx, approval_rx) = if approvals_enabled {
        let (tx, rx) = tokio::sync::mpsc::channel(limits.approval_queue_capacity.max(1));
        (Some(tx), Some(Arc::new(AsyncMutex::new(rx))))
    } else {
        (None, None)
    };

    let mut workers = tokio::task::JoinSet::new();
    let config = Arc::new(config);
    for _ in 0..limits.request_workers.max(1) {
        workers.spawn(run_firewall_request_worker(
            conn.clone(),
            Arc::clone(&request_rx),
            Arc::clone(&config),
            egress_approver.clone(),
            approved_domains.clone(),
            Arc::clone(&dns_resolver),
            dns_cache.clone(),
            approval_tx.clone(),
            cancel.clone(),
        ));
    }
    if let Some(approval_rx) = approval_rx {
        for _ in 0..limits.approval_workers.max(1) {
            workers.spawn(run_firewall_approval_worker(
                Arc::clone(&approval_rx),
                cancel.clone(),
                limits.approval_timeout,
            ));
        }
    }

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            joined = workers.join_next(), if !workers.is_empty() => {
                if cancel.is_cancelled() {
                    break;
                }
                match joined {
                    Some(Ok(())) => panic!("egress firewall worker exited while its Host was live"),
                    Some(Err(error)) => panic!("egress firewall worker died: {error}"),
                    None => panic!("egress firewall worker registry became empty"),
                }
            }
            ev = attached_rx.recv() => {
                let Some(ev) = ev else { break };
                // Keep the reliable subscription's byte/count token attached
                // while this event waits in our bounded queue. Deserializing
                // here would transfer its heap into an unaccounted job.
                let job = FirewallRequestJob::EnableSession(ev);
                if let Err(error) = request_tx.try_send(job) {
                    reject_saturated_firewall_job(&conn, &cancel, error.into_inner()).await;
                    break;
                }
            }
            ev = paused_rx.recv() => {
                let Some(ev) = ev else { break };
                let job = FirewallRequestJob::Paused(ev);
                if let Err(error) = request_tx.try_send(job) {
                    reject_saturated_firewall_job(&conn, &cancel, error.into_inner()).await;
                    break;
                }
            }
        }
    }

    cancel.cancel();
    drop(request_tx);
    drop(approval_tx);
    workers.abort_all();
    while workers.join_next().await.is_some() {}
}

#[allow(clippy::too_many_arguments)]
async fn run_firewall_request_worker(
    conn: Connection,
    request_rx: Arc<AsyncMutex<tokio::sync::mpsc::Receiver<FirewallRequestJob>>>,
    config: Arc<crate::firewall::FirewallConfig>,
    egress_approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
    approved_domains: crate::firewall::ApprovedDomains,
    dns_resolver: Arc<dyn crate::firewall::HostResolver>,
    dns_cache: crate::firewall::DnsResolverCache,
    approval_tx: Option<tokio::sync::mpsc::Sender<FirewallApprovalJob>>,
    cancel: CancellationToken,
) {
    loop {
        let mut receiver = tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            receiver = request_rx.lock() => receiver,
        };
        let job = tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            job = receiver.recv() => job,
        };
        drop(receiver);
        let Some(job) = job else { return };

        match job {
            FirewallRequestJob::EnableSession(event) => {
                let Ok(attached) =
                    serde_json::from_value::<EventAttachedToTarget>(event.params)
                else {
                    tracing::error!(
                        target: "nomi_browser_engine::backend::cdp",
                        "malformed attachedToTarget event reached the armed firewall; closing connection fail-closed"
                    );
                    conn.shutdown().await;
                    return;
                };
                let session_id: String = attached.session_id.into();
                let target_type = attached.target_info.r#type;
                let result = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    result = enable_fetch_on_session(&conn, &session_id) => result,
                };
                if let Err(error) = result {
                    tracing::warn!(
                        target: "nomi_browser_engine::backend::cdp",
                        %error, %session_id, %target_type,
                        "Fetch.enable on attached session failed; egress firewall has a gap for this target"
                    );
                } else {
                    tracing::debug!(
                        target: "nomi_browser_engine::backend::cdp",
                        %session_id, %target_type,
                        "Fetch.enable armed on attached session (egress firewall)"
                    );
                }
            }
            FirewallRequestJob::Paused(event) => {
                let session_id = event.session_id;
                let Ok(paused) = serde_json::from_value::<EventRequestPaused>(event.params) else {
                    tracing::error!(
                        target: "nomi_browser_engine::backend::cdp",
                        "malformed requestPaused event cannot be safely released; closing connection fail-closed"
                    );
                    conn.shutdown().await;
                    return;
                };
                let approval = handle_paused_request(
                    &conn,
                    config.as_ref(),
                    egress_approver.as_ref(),
                    &approved_domains,
                    &session_id,
                    paused,
                    dns_resolver.as_ref(),
                    &dns_cache,
                    &cancel,
                )
                .await;
                let Some(approval) = approval else { continue };
                let Some(approval_tx) = approval_tx.as_ref() else {
                    reject_saturated_approval(approval, &cancel, "approval executor unavailable")
                        .await;
                    continue;
                };
                if let Err(error) = approval_tx.try_send(approval) {
                    reject_saturated_approval(
                        error.into_inner(),
                        &cancel,
                        "approval executor saturated",
                    )
                    .await;
                }
            }
        }
    }
}

async fn run_firewall_approval_worker(
    approval_rx: Arc<AsyncMutex<tokio::sync::mpsc::Receiver<FirewallApprovalJob>>>,
    cancel: CancellationToken,
    approval_timeout: Duration,
) {
    loop {
        let mut receiver = tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            receiver = approval_rx.lock() => receiver,
        };
        let job = tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            job = receiver.recv() => job,
        };
        drop(receiver);
        let Some(job) = job else { return };
        resolve_firewall_approval(job, &cancel, approval_timeout).await;
    }
}

async fn reject_saturated_firewall_job(
    conn: &Connection,
    cancel: &CancellationToken,
    job: FirewallRequestJob,
) {
    tracing::error!(
        target: "nomi_browser_engine::backend::cdp",
        request_queue_capacity = FIREWALL_REQUEST_QUEUE_CAPACITY,
        "egress firewall request executor saturated; failing closed and closing the Host connection"
    );
    if let FirewallRequestJob::Paused(event) = job {
        let session_id = event.session_id;
        let Ok(paused) = serde_json::from_value::<EventRequestPaused>(event.params) else {
            conn.shutdown().await;
            return;
        };
        let rejected = tokio::time::timeout(
            FIREWALL_OVERFLOW_REJECT_TIMEOUT,
            fetch_fail(conn, &session_id, paused.request_id, cancel),
        )
        .await;
        if !matches!(rejected, Ok(true)) {
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                "could not prove the saturated request was rejected within budget; closing connection"
            );
        }
    }
    conn.shutdown().await;
}

async fn reject_saturated_approval(
    job: FirewallApprovalJob,
    cancel: &CancellationToken,
    reason: &'static str,
) {
    let FirewallApprovalJob {
        conn,
        session_id,
        request_id,
        preview,
        ..
    } = job;
    tracing::warn!(
        target: "nomi_browser_engine::backend::cdp",
        target_host = %preview.host,
        approval_queue_capacity = FIREWALL_APPROVAL_QUEUE_CAPACITY,
        %reason,
        "egress approval admission rejected; failing the gated request closed"
    );
    if !matches!(
        tokio::time::timeout(
        FIREWALL_OVERFLOW_REJECT_TIMEOUT,
        fetch_fail(&conn, &session_id, request_id, cancel),
    )
        .await,
        Ok(true)
    )
        && !cancel.is_cancelled()
    {
        conn.shutdown().await;
    }
}

async fn resolve_firewall_approval(
    job: FirewallApprovalJob,
    cancel: &CancellationToken,
    approval_timeout: Duration,
) {
    let FirewallApprovalJob {
        conn,
        session_id,
        request_id,
        target,
        preview,
        approver,
        approved_domains,
    } = job;
    let verdict = tokio::select! {
        biased;
        _ = cancel.cancelled() => return,
        result = tokio::time::timeout(approval_timeout, approver.approve_egress(&preview)) => {
            match result {
                Ok(verdict) => verdict,
                Err(_) => {
                    tracing::warn!(
                        target: "nomi_browser_engine::backend::cdp",
                        target_host = %preview.host,
                        "egress approval timed out — failing closed (rejecting the gated request)"
                    );
                    crate::firewall::EgressVerdict::Fail
                }
            }
        }
    };
    if verdict.is_continue() {
        if verdict.remembers_domain() {
            approved_domains.record(&target);
        }
        fetch_continue(&conn, &session_id, request_id, cancel).await;
    } else {
        fetch_fail(&conn, &session_id, request_id, cancel).await;
    }
}

/// **防火墙 watchdog（fail-closed）**：监视出口防火墙循环的 `JoinHandle`，任务
/// **非 abort 死亡**（panic 逃出循环，如经注入的 `EgressApprover` trait 对象逃出
/// `handle_paused_request`）时把**整条 CDP 连接 fail 掉**（`Connection::shutdown`）。
///
/// 理由：防火墙循环一死，其 `Fetch.requestPaused` 可靠订阅接收端即被 drop——
/// 已 arm 的 session 的被拦请求从此无人应答（永久悬挂），新 session 则会被
/// transport 的粘性 arm 标记 fail-closed 拒绝放行。二者都不可恢复，唯一诚实的
/// 处置是**立即 fail 整条连接**：挂起命令全部解除为 `Closed`、pipe 运输下
/// Chromium 读到 EOF 自退。恢复路径 = 重启 host。
///
/// 正常退出（`Ok(())`，循环因连接关闭而结束）与**主动 abort**（shutdown/Drop 路径）
/// **不**触发 fail——只有意外死亡才算。
fn spawn_firewall_watchdog(
    conn: Connection,
    firewall_loop: tokio::task::JoinHandle<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match firewall_loop.await {
            // 循环自然退出：可靠通道已关（连接已 fail/关闭），无需再动作。
            Ok(()) => {}
            // 主动 abort（shutdown/Drop 编排）：非死亡，勿 fail 连接。
            Err(join_error) if join_error.is_cancelled() => {}
            // 意外死亡（panic 逃出循环）→ fail-closed：fail 整条连接。
            Err(join_error) => {
                tracing::error!(
                    target: "nomi_browser_engine::backend::cdp",
                    error = %join_error,
                    "egress firewall task died unexpectedly; failing the CDP connection closed \
                     (relaunch the browser host to recover)"
                );
                conn.shutdown().await;
            }
        }
    })
}

/// 处理一条 `Fetch.requestPaused`：抽取判定所需输入 → [`crate::firewall::decide`] → dispatch
/// continue/fail/（D2）**悬挂等审批**。**绝不**让请求**无条件**悬挂（Allow/Block 立即 continue/fail；
/// GatePost 走 D2 悬挂机制，仍有界——超时即 fail-closed，绝不永久挂起）。
///
/// **P3-D2（裁决④/决策3）+ F6 白屏回归修**：[`crate::firewall::FirewallDecision::GatePost`] 的处置
/// 按审批通道有无分岔：
/// 1. 先查 `approved_domains`（决策3 always_allow 记住域）——目标 eTLD+1 已被本会话批准 → **直接
///    continue**（不再悬挂审批）；
/// 2. **无审批通道（`egress_approver=None`，托管上下文）→ 放行 + warn 留痕**（E5 pre-approval 姿态：
///    「检测+留痕，审批接线前放行」）。托管 host 无人在回路可批——fail-closed 只会把被访问站点的正常
///    出口打成 BlockedByClient 白屏（F6 回归）。审计记 host/size/字段名（绝不含值）。**硬 Block
///    （SSRF IP 封禁 / DNS 守卫 / deny_etld1）不经 GatePost，仍 failRequest fail-closed**；
/// 3. **有审批通道（standalone/桌面接管路径）→ 悬挂**该请求（保留 `request_id`，**不**立即
///    continue/fail）+ 移交 Host 所有的固定并发、有界队列审批执行器（事件循环立即回到
///    `select!` 继续 pump，**绝不**在此同步阻塞）；已登记 worker `await`
///    [`crate::firewall::EgressApprover`]（带
///    [`crate::firewall::EGRESS_APPROVAL_TIMEOUT`] 超时）取裁决 → 批准 `continueRequest`（可选记住域）/
///    拒绝/超时 `failRequest`（**fail-closed**，闭合 P2 泄漏窗口）。
// egress firewall 上下文参数较多（config/approver/approved_domains/resolver/cache）；SD-5 接入真实
// egress approver 时再收拢成一个 EgressContext 结构体（届时参数更多，结构体更划算），此处先 allow。
#[allow(clippy::too_many_arguments)]
async fn handle_paused_request(
    conn: &Connection,
    config: &crate::firewall::FirewallConfig,
    egress_approver: Option<&Arc<dyn crate::firewall::EgressApprover>>,
    approved_domains: &crate::firewall::ApprovedDomains,
    session_id: &str,
    paused: EventRequestPaused,
    dns_resolver: &dyn crate::firewall::HostResolver,
    dns_cache: &crate::firewall::DnsResolverCache,
    cancel: &CancellationToken,
) -> Option<FirewallApprovalJob> {
    let request_id = paused.request_id.clone();
    let url = paused.request.url.clone();
    let method = paused.request.method.clone();

    // 当前页 origin：优先请求自带的 `Origin` 头（浏览器对跨域写请求会带），退而求其次 `Referer`。
    // 这就是「发起请求的文档」的 origin——跨域判定的左侧。两者都没有（同源导航 / 顶层 nav）→ 用目标
    // URL 自身（同 host 比较恒非跨域，等价于「不门控顶层同站导航」，合理）。
    let headers = &paused.request.headers;
    let current_origin = header_value(headers, "Origin")
        .or_else(|| header_value(headers, "origin"))
        .or_else(|| header_value(headers, "Referer"))
        .or_else(|| header_value(headers, "referer"))
        .unwrap_or_else(|| url.clone());
    let content_type = header_value(headers, "Content-Type")
        .or_else(|| header_value(headers, "content-type"));

    // body：从 post_data_entries 解 base64（Fetch.requestPaused 在 Request 阶段通常带 entries）。
    let body = decode_post_data_entries(&paused);
    let has_post_data = paused.request.has_post_data.unwrap_or(false) || body.is_some();

    // 目标 host 若**本身是 IP 字面量**（最危险的 SSRF 形态：直接拿内网/元数据 IP 当 URL）→ 同步判 IP
    // 封禁，无需 DNS。域名 host 的异步 DNS→IP 路径见 TODO（E5 同步覆盖 IP 字面量这一主面）。
    let target_host = nomifun_secret::host_of(&url);
    let resolved_ip = target_host
        .as_deref()
        .and_then(crate::firewall::ip_literal_of_host);

    // 是否顶层 Document 导航（resourceType==Document）。域名 allowlist 出口门控对顶层导航豁免——allowlist
    // 是出口/数据外泄控制（限制跨域子请求把数据发往哪），不是导航监狱；agent 导航到一个 URL 是意图行为，
    // 不该因「注册 secret → allowlist 非空」被关进白名单域。**仅豁免 allow 白名单**：IP 封禁（SSRF/元数据）
    // 对导航仍生效、deny 黑名单对导航仍硬拦、跨域 POST 门控不受影响（见 firewall::decide / domain_policy）。
    let is_top_level_navigation = paused.resource_type == ResourceType::Document;

    // ─── SD-1: DNS→IP SSRF guard（egress 子资源）────────────────────────────────
    // 对 **egress 子资源请求**（非顶层 Document 导航），当 `block_private_ips` 开且目标 host 是域名
    // （非 IP 字面量）→ 异步 DNS 解析 + 检查所有 resolved IPs 是否命中 is_blocked_ip。
    // ANY 命中 / 解析失败 → fail-closed (Block)。**仅 egress-only**（不拦顶层导航——与 allowlist 豁免同理，
    // 但 IP 字面量的同步判定对 top-nav 仍生效，见 decide 的 IP 封禁档）。
    if config.block_private_ips && !is_top_level_navigation && resolved_ip.is_none() {
        // resolved_ip==None 意味 host 不是 IP 字面量（是域名）→ 需 DNS 解析。
        let blocked = if let Some(host) = target_host.as_deref() {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return None,
                blocked = crate::firewall::check_dns_ssrf(host, dns_resolver, dns_cache) => blocked,
            }
        } else {
            false
        };
        if blocked {
            let host = target_host.as_deref().unwrap_or_default();
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                url = %url, host = %host,
                "egress firewall BLOCKED request: domain resolves to private/metadata IP (DNS→IP SSRF guard)"
            );
            fetch_fail(conn, session_id, request_id, cancel).await;
            return None;
        }
    }

    let decision = crate::firewall::decide(
        config,
        &crate::firewall::RequestInfo {
            resolved_ip,
            method: &method,
            has_post_data,
            body: body.as_deref(),
            content_type: content_type.as_deref(),
            current_origin: &current_origin,
            target_url: &url,
            is_top_level_navigation,
        },
    );

    match decision {
        crate::firewall::FirewallDecision::Allow => {
            fetch_continue(conn, session_id, request_id, cancel).await;
            None
        }
        crate::firewall::FirewallDecision::Block { reason } => {
            // 硬阻断（IP 封禁，SSRF 防护）。failRequest{BlockedByClient}。
            tracing::warn!(
                target: "nomi_browser_engine::backend::cdp",
                url = %url, reason = %reason,
                "egress firewall BLOCKED request (failRequest)"
            );
            fetch_fail(conn, session_id, request_id, cancel).await;
            None
        }
        crate::firewall::FirewallDecision::GatePost { preview } => {
            // P3-D2（裁决④/决策3）：闭合 P2 跨域 POST 泄漏窗口——不再 detect-but-continue。
            //
            // ① 决策3 always_allow：目标 eTLD+1 已被本会话批准（用户此前审批时选「记住此域」）→
            //    直接放行（不再悬挂审批）。同域后续提交不再反复弹。
            if approved_domains.is_approved(&url) {
                tracing::debug!(
                    target: "nomi_browser_engine::backend::cdp",
                    target_host = %preview.host,
                    "egress firewall: gated request to an already-approved domain (always_allow) — continuing"
                );
                fetch_continue(conn, session_id, request_id, cancel).await;
                return None;
            }

            // ② **无审批通道（托管上下文）→ 检测 + 留痕后放行（E5 pre-approval 姿态；F6 白屏回归修）**。
            //    托管 host 的模板 EngineConfig 不接线 EgressApprover（egress_approver=None，无人在回路
            //    可批）——此时把 GatePost fail-closed 会让「域 allowlist 外的子资源出口 / 跨域 POST」
            //    直接 BlockedByClient 白屏（注册任一 secret 即毁掉所有托管浏览）。产品红线是浏览顺滑
            //    零打断：GatePost（软「升审批」档）在无审批通道时降级为 **continueRequest + warn 留痕**
            //    （审计 host/size/字段名——绝不含字段值）。**硬 Block 不受影响**：SSRF IP 封禁（decide
            //    的 Block 臂 + 上方 DNS 守卫 early-return）与 deny_etld1 黑名单仍 failRequest
            //    （fail-closed）——只有 GatePost 这一档在无通道时放行留痕。
            let Some(approver) = egress_approver else {
                tracing::warn!(
                    target: "nomi_browser_engine::backend::cdp",
                    url = %url,
                    target_host = %preview.host, body_size = preview.size,
                    field_names = ?preview.field_names, // 仅字段名（绝不含值）
                    "egress firewall gated egress but no approval channel is wired (managed context) — \
                     allowing with audit trail (E5 pre-approval posture; SSRF/denylist hard blocks unaffected)"
                );
                fetch_continue(conn, session_id, request_id, cancel).await;
                return None;
            };

            // ③ 有审批通道（standalone/桌面接管路径）：悬挂该请求等人在回路裁决。**绝不**在此 CDP 事件
            //    handler 里同步阻塞（会卡死整个防火墙事件循环——所有 session 的 requestPaused/
            //    attachedToTarget 都经它）。故把 request_id 保留（不 continue/不 fail），交给 Host
            //    的有界审批 worker 去 await 审批 → 据裁决 continue/fail。审批超时 / 拒绝 → **fail-closed**
            //    （failRequest）。预览只 host/size/字段名（绝不含值，复用 E5 build_post_preview）。
            tracing::info!(
                target: "nomi_browser_engine::backend::cdp",
                target_host = %preview.host, body_size = preview.size,
                field_names = ?preview.field_names, // 仅字段名（绝不含值）
                "egress firewall gated cross-origin POST / off-allowlist egress — suspending for out-of-band approval (fail-closed on timeout/deny)"
            );

            // 把请求移交给 Host 所有的固定审批 worker 池。调用者用 `try_send` 做有界
            // admission；饱和时立即 failRequest，绝不创建 detached per-request task。
            Some(FirewallApprovalJob {
                conn: conn.clone(),
                session_id: session_id.to_string(),
                request_id,
                target: preview.host.clone(),
                preview,
                approver: Arc::clone(approver),
                approved_domains: approved_domains.clone(),
            })
        }
    }
}

/// 从 `Headers`（`serde_json::Value` object）取某头的值（精确 key 匹配；调用方自己试大小写变体）。
fn header_value(
    headers: &chromiumoxide::cdp::browser_protocol::network::Headers,
    key: &str,
) -> Option<String> {
    headers
        .inner()
        .as_object()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 从 `Fetch.requestPaused` 的 `request.post_data_entries`（base64 `bytes`）解出 body 字节。
/// 无 entries / 解码失败 → `None`（判定层据 `has_post_data` 仍可判「有 body 但内容不可见」）。
fn decode_post_data_entries(paused: &EventRequestPaused) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let entries = paused.request.post_data_entries.as_ref()?;
    let mut out: Vec<u8> = Vec::new();
    for e in entries {
        if let Some(bin) = &e.bytes {
            let s: &str = bin.as_ref();
            // CDP post data entry bytes 是 base64 编码。解码失败的 entry 跳过（best-effort）。
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(s) {
                out.extend_from_slice(&decoded);
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 放行被拦请求（`Fetch.continueRequest`）。best-effort：失败只 debug（请求会因 CDP 超时自处理）。
async fn fetch_continue(
    conn: &Connection,
    session_id: &str,
    request_id: chromiumoxide::cdp::browser_protocol::fetch::RequestId,
    cancel: &CancellationToken,
) {
    let params = ContinueRequestParams::new(request_id);
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return,
        result = conn.send_may_fail::<ContinueRequestParams>(session_id, &params) => result,
    };
    if let Err(e) = result {
        tracing::debug!(
            target: "nomi_browser_engine::backend::cdp",
            error = %e, "Fetch.continueRequest failed (benign; request may have already resolved)"
        );
    }
}

/// 阻断被拦请求（`Fetch.failRequest{BlockedByClient}`）。best-effort：失败只 debug。
async fn fetch_fail(
    conn: &Connection,
    session_id: &str,
    request_id: chromiumoxide::cdp::browser_protocol::fetch::RequestId,
    cancel: &CancellationToken,
) -> bool {
    let params = FailRequestParams::new(request_id, NetworkErrorReason::BlockedByClient);
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return false,
        result = conn.send_may_fail::<FailRequestParams>(session_id, &params) => result,
    };
    if let Err(e) = result {
        tracing::debug!(
            target: "nomi_browser_engine::backend::cdp",
            error = %e, "Fetch.failRequest failed (benign; request may have already resolved)"
        );
        false
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_html_renderer_and_rust_guards_share_one_utf8_limit() {
        let expression = bounded_rendered_html_expression();
        let content_limit = crate::engine::MAX_RENDERED_HTML_BYTES
            - crate::engine::RENDERED_HTML_TRUNCATION_MARKER.len();
        assert!(expression.contains(&format!("const limit = {content_limit}")));
        assert!(expression.contains("codePointAt"));

        let hostile = format!("<html>{}</html>", "😀".repeat(content_limit / 4 + 64));
        let html = rendered_html_from_renderer_value(&serde_json::json!({
            "text": hostile,
            "truncated": false,
        }));
        assert!(html.len() <= crate::engine::MAX_RENDERED_HTML_BYTES);
        assert!(html.ends_with(crate::engine::RENDERED_HTML_TRUNCATION_MARKER));
        assert!(std::str::from_utf8(html.as_bytes()).is_ok());
    }

    #[test]
    fn rendered_html_preserves_small_unicode_documents_without_marker() {
        let html = rendered_html_from_renderer_value(&serde_json::json!({
            "text": "<html><body>你好😀</body></html>",
            "truncated": false,
        }));
        assert_eq!(html, "<html><body>你好😀</body></html>");
        assert!(!html.ends_with(crate::engine::RENDERED_HTML_TRUNCATION_MARKER));
    }

    struct CountingTaskTabReservation {
        drops: Arc<AtomicUsize>,
    }

    impl TaskTabReservation for CountingTaskTabReservation {}

    impl Drop for CountingTaskTabReservation {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct RejectingTaskTabAuthority;

    #[async_trait]
    impl TaskTabReservationAuthority for RejectingTaskTabAuthority {
        async fn reserve(
            &self,
            _task_resource_key: &str,
            _lane_id: &str,
            _reservation_key: &str,
        ) -> Result<Arc<dyn TaskTabReservation>, BrowserError> {
            Err(BrowserError::Blocked {
                reason: "test task tab capacity reached".into(),
            })
        }
    }

    struct BoundedTaskTabAuthority {
        live: Arc<AtomicUsize>,
        max: usize,
    }

    struct BoundedTaskTabReservation {
        live: Arc<AtomicUsize>,
    }

    impl TaskTabReservation for BoundedTaskTabReservation {}

    impl Drop for BoundedTaskTabReservation {
        fn drop(&mut self) {
            let previous = self.live.fetch_sub(1, Ordering::SeqCst);
            debug_assert!(previous > 0, "task tab reservation count underflow");
        }
    }

    #[async_trait]
    impl TaskTabReservationAuthority for BoundedTaskTabAuthority {
        async fn reserve(
            &self,
            _task_resource_key: &str,
            _lane_id: &str,
            _reservation_key: &str,
        ) -> Result<Arc<dyn TaskTabReservation>, BrowserError> {
            self.live
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |live| {
                    (live < self.max).then_some(live + 1)
                })
                .map_err(|_| BrowserError::Blocked {
                    reason: "test task tab capacity reached".into(),
                })?;
            Ok(Arc::new(BoundedTaskTabReservation {
                live: Arc::clone(&self.live),
            }))
        }
    }

    #[test]
    fn per_lane_tab_capacity_is_bounded() {
        assert!(tab_capacity_available(0));
        assert!(tab_capacity_available(MAX_TABS_PER_LANE - 1));
        assert!(!tab_capacity_available(MAX_TABS_PER_LANE));
        assert!(!tab_capacity_available(MAX_TABS_PER_LANE + 1));
    }

    #[test]
    fn closed_target_bookkeeping_stays_constant_under_one_hundred_thousand_tab_churn() {
        let mut state = HostRouteState::default();
        for index in 0..100_000usize {
            let target_id = format!("target-{index}");
            let frame_id = format!("frame-{index}");
            let session_id = format!("session-{index}");
            state.ownership.claim("long-lived-lane", &target_id).unwrap();
            state
                .retired_target_owner
                .insert(target_id.clone(), "long-lived-lane".into());
            state.cleanup_inflight.insert(target_id.clone());
            state
                .lost_targets
                .insert(target_id.clone(), tokio::time::Instant::now());
            state
                .session_targets
                .insert(session_id, target_id.clone());
            state
                .frame_owner
                .insert(frame_id.clone(), "long-lived-lane".into());

            state.release_target_bookkeeping(&target_id, Some(&frame_id));
        }

        assert!(state.ownership.targets_for_lane("long-lived-lane").is_empty());
        assert!(state.retired_target_owner.is_empty());
        assert!(state.cleanup_inflight.is_empty());
        assert!(state.lost_targets.is_empty());
        assert!(state.session_targets.is_empty());
        assert!(state.frame_owner.is_empty());
    }

    #[test]
    fn per_tab_oopif_capacity_is_bounded() {
        assert!(oopif_capacity_available(0));
        assert!(oopif_capacity_available(MAX_OOPIFS_PER_TAB - 1));
        assert!(!oopif_capacity_available(MAX_OOPIFS_PER_TAB));
        assert!(!oopif_capacity_available(MAX_OOPIFS_PER_TAB + 1));
    }

    #[test]
    fn per_host_download_route_capacity_is_bounded() {
        assert!(download_capacity_available(0));
        assert!(download_capacity_available(
            MAX_PENDING_DOWNLOADS_PER_HOST - 1
        ));
        assert!(!download_capacity_available(
            MAX_PENDING_DOWNLOADS_PER_HOST
        ));
        assert!(!download_capacity_available(
            MAX_PENDING_DOWNLOADS_PER_HOST + 1
        ));
    }

    #[cfg(any(windows, unix))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_direct_finishes_and_handoff_publish_one_sticky_completion() {
        let claimed = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let hooks = Arc::new(DurableProcessCleanupTestHooks {
            handoff_claimed: Some(Arc::clone(&claimed)),
            handoff_release: Some(Arc::clone(&release)),
            ..DurableProcessCleanupTestHooks::default()
        });
        let (_temp, profile, cleanup, pid) =
            durable_process_cleanup_fixture("finish-handoff-race", Some(Arc::clone(&hooks)))
                .await;
        let process = cleanup
            .process
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned()
            .expect("fixture starts with exact process authority");
        let process_references_before_finish = Arc::strong_count(&process);
        let held_process = process.lock().await;

        let first_cleanup = Arc::clone(&cleanup);
        let first_finish = tokio::spawn(async move { first_cleanup.finish().await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while hooks.finish_gate_entries.load(Ordering::Acquire) != 1
                || Arc::strong_count(&process) <= process_references_before_finish
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first finish owns the gate and snapshots process authority");

        let handoff_cleanup = Arc::clone(&cleanup);
        let handoff = std::thread::spawn(move || handoff_cleanup.hand_off());
        claimed.wait();
        assert_eq!(
            cleanup.state.load(Ordering::Acquire),
            1,
            "handoff claims relay state before direct cleanup publishes completion"
        );

        let second_cleanup = Arc::clone(&cleanup);
        let second_finish = tokio::spawn(async move { second_cleanup.finish().await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while hooks.finish_calls.load(Ordering::Acquire) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second finish reaches the per-cleanup serialization gate");
        assert_eq!(
            hooks.finish_gate_entries.load(Ordering::Acquire),
            1,
            "a second finisher cannot observe pre-completion process/ticket state"
        );
        assert!(
            !second_finish.is_finished(),
            "the concurrent finisher remains behind the first finisher"
        );

        drop(held_process);
        assert!(
            tokio::time::timeout(Duration::from_secs(10), first_finish)
                .await
                .expect("first direct finish cannot hang")
                .expect("first direct finish task joins")
                .is_ok(),
            "first direct finish proves exact cleanup"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), second_finish)
                .await
                .expect("serialized second finish cannot lose completion")
                .expect("second direct finish task joins")
                .is_ok(),
            "second direct finish observes sticky success"
        );
        assert_eq!(cleanup.state.load(Ordering::Acquire), 2);
        assert!(
            cleanup
                .process
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none(),
            "successful direct finish removes local process authority"
        );

        // Recreate the former lost-wake tail exactly: handoff has already won
        // state 0->1, but resumes only after direct finish took the process.
        // It therefore has no process and publishes no ticket.
        release.wait();
        handoff.join().expect("racing handoff thread joins");
        assert!(
            cleanup.handoff_ticket.borrow().is_none(),
            "handoff which loses the process race has no relay ticket to publish"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(1), cleanup.finish())
                .await
                .expect("completion-before-wait returns immediately")
                .is_ok(),
            "late finish observes sticky direct completion without a ticket"
        );

        wait_for_durable_cleanup_process_exit(pid).await;
        assert!(
            !profile.exists(),
            "exact direct cleanup removes marker, port file, and ephemeral profile"
        );
    }

    #[cfg(any(windows, unix))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_after_handoff_waits_for_or_takes_over_exact_cleanup() {
        let (_temp, profile, cleanup, pid) =
            durable_process_cleanup_fixture("finish-after-handoff", None).await;

        cleanup.hand_off();
        assert_eq!(
            cleanup.state.load(Ordering::Acquire),
            1,
            "handoff owns the exact cleanup relay before finish starts"
        );
        assert!(
            cleanup
                .process
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none(),
            "handoff transfers rather than duplicates local process authority"
        );

        let first = {
            let cleanup = Arc::clone(&cleanup);
            tokio::spawn(async move { cleanup.finish().await })
        };
        let second = {
            let cleanup = Arc::clone(&cleanup);
            tokio::spawn(async move { cleanup.finish().await })
        };
        let (first, second) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(first, second)
        })
        .await
        .expect("two finish calls after handoff cannot hang");
        assert!(
            first.expect("first post-handoff finish joins").is_ok(),
            "first post-handoff finish proves exact cleanup"
        );
        assert!(
            second.expect("second post-handoff finish joins").is_ok(),
            "second post-handoff finish observes the same sticky completion"
        );
        assert_eq!(cleanup.state.load(Ordering::Acquire), 2);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), cleanup.finish())
                .await
                .expect("late post-handoff finish returns immediately")
                .is_ok()
        );

        wait_for_durable_cleanup_process_exit(pid).await;
        assert!(
            !profile.exists(),
            "handoff cleanup removes the exact ephemeral profile"
        );
    }

    #[cfg(any(windows, unix))]
    async fn durable_process_cleanup_fixture(
        name: &str,
        test_hooks: Option<Arc<DurableProcessCleanupTestHooks>>,
    ) -> (
        tempfile::TempDir,
        PathBuf,
        Arc<DurableProcessCleanup>,
        u32,
    ) {
        let temp = tempfile::tempdir().expect("create durable cleanup fixture root");
        let profile = temp.path().join(name);
        std::fs::create_dir_all(&profile).expect("create durable cleanup fixture profile");
        let claim = crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("claim durable cleanup fixture profile");
        let (process, executable) = spawn_durable_cleanup_process_fixture();
        let token = crate::profile::write_browser_ownership_marker(
            &claim,
            &profile,
            &executable,
            process.child(),
            None,
        )
        .await
        .expect("commit durable cleanup fixture ownership");
        let pid = process.id().expect("durable cleanup fixture pid");
        std::fs::write(
            profile.join("DevToolsActivePort"),
            b"9222\n/devtools/browser/durable-cleanup\n",
        )
        .expect("write durable cleanup port artifact");
        drop(claim);

        let mut cleanup = DurableProcessCleanup::new(
            process,
            LaunchedProfileCleanupAuthority::new(Some(profile.clone())),
            token,
            None,
        );
        cleanup.test_hooks = test_hooks;
        (temp, profile, Arc::new(cleanup), pid)
    }

    #[cfg(any(windows, unix))]
    fn spawn_durable_cleanup_process_fixture(
    ) -> (
        nomi_process_runtime::ManagedChildProcess,
        PathBuf,
    ) {
        #[cfg(windows)]
        {
            let shell = PathBuf::from(
                std::env::var_os("COMSPEC").expect("Windows COMSPEC identifies cmd.exe"),
            );
            let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&shell);
            builder
                .args(["/D", "/S", "/C", "ping -n 60 127.0.0.1 >NUL"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            return (
                builder.spawn_managed().expect("spawn Windows cleanup fixture"),
                shell,
            );
        }

        #[cfg(unix)]
        {
            // Spawn the sleeper directly: a `/bin/sh -c` wrapper may exec into
            // sleep, so the committed ownership marker would name a different
            // executable than the live process image (darwin rejects that).
            let executable = PathBuf::from("/bin/sleep");
            let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&executable);
            builder
                .arg("60")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            (
                builder.spawn_managed().expect("spawn Unix cleanup fixture"),
                executable,
            )
        }
    }

    #[cfg(any(windows, unix))]
    async fn wait_for_durable_cleanup_process_exit(pid: u32) {
        tokio::time::timeout(Duration::from_secs(10), async move {
            loop {
                use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
                if system.process(sysinfo::Pid::from_u32(pid)).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("durable cleanup fixture process exits within deadline");
    }

    #[tokio::test]
    async fn cleanup_executor_start_failure_is_fail_closed_before_host_publication() {
        let (connection, server) = close_target_fake_connection(Vec::new()).await;
        TARGET_CLEANUP_EXECUTOR_START_FAILURE.with(|failure| failure.set(true));
        assert!(
            TargetCleanupExecutor::new(connection.clone(), None).is_err(),
            "a Host without its bounded cleanup executor must not be constructed"
        );
        let executor = TargetCleanupExecutor::new(connection.clone(), None)
            .expect("the scoped one-shot failure must not leak");
        assert_eq!(executor.worker_count(), 1);
        drop(executor);
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn initial_page_target_is_foreground_only_for_effective_headful_mode() {
        assert!(
            !page_target_should_start_in_background(true, true),
            "an explicitly headful Host with a live display must publish a visible target"
        );
        assert!(page_target_should_start_in_background(false, true));
        assert!(page_target_should_start_in_background(true, false));
        assert!(page_target_should_start_in_background(false, false));
    }

    #[test]
    fn initial_page_target_params_preserve_foreground_request_on_the_cdp_wire() {
        let foreground =
            serde_json::to_value(initial_page_target_params(false)).expect("serialize params");
        let background =
            serde_json::to_value(initial_page_target_params(true)).expect("serialize params");

        assert_eq!(foreground["url"], "about:blank");
        assert_eq!(foreground["background"], false);
        assert_eq!(background["background"], true);
    }

    #[test]
    fn managed_host_constructor_cannot_accept_external_profile_cleanup_authority() {
        fn assert_signature<F, Fut>(_: F)
        where
            F: FnOnce(
                Launched,
                bool,
                bool,
                Option<PathBuf>,
                Option<String>,
                crate::firewall::FirewallConfig,
                Option<Arc<dyn crate::firewall::EgressApprover>>,
                Option<serde_json::Value>,
                Option<Arc<dyn crate::firewall::HostResolver>>,
            ) -> Fut,
        {
        }

        // This compile-time signature assertion intentionally has no profile
        // path argument. A stable Launched(None) therefore cannot be paired
        // with an injected Some(path), and a mismatched path is inexpressible.
        assert_signature(CdpHostRuntime::from_launched);
    }

    #[test]
    fn launched_profile_cleanup_authority_preserves_stable_and_exact_modes() {
        assert!(
            LaunchedProfileCleanupAuthority::new(None)
                .into_profile_dir()
                .is_none(),
            "stable launch authority must remain marker-only cleanup"
        );

        let exact_profile = PathBuf::from("exact-ephemeral-profile");
        assert_eq!(
            LaunchedProfileCleanupAuthority::new(Some(exact_profile.clone()))
                .into_profile_dir(),
            Some(exact_profile),
            "ephemeral cleanup must preserve the exact launch-authorized path"
        );
    }

    #[test]
    fn cleanup_inventory_validation_rejects_every_malformed_target_info_shape() {
        let malformed = [
            serde_json::json!({}),
            serde_json::json!({"targetId": 7, "type": "page"}),
            serde_json::json!({"targetId": "target-a"}),
            serde_json::json!({"targetId": "target-a", "type": null}),
            serde_json::json!({"targetId": "target-a", "type": "page", "openerId": null}),
            serde_json::json!({"targetId": "target-a", "type": "page", "openerId": 9}),
        ];
        for target_info in malformed {
            let result =
                validated_target_inventory(&serde_json::json!({"targetInfos": [target_info]}));
            assert!(
                result.is_err(),
                "malformed TargetInfo must never contribute to an absence proof"
            );
        }
        assert!(
            validated_target_inventory(&serde_json::json!({
                "targetInfos": [{
                    "targetId": "target-a",
                    "type": "page",
                    "openerId": "target-parent"
                }]
            }))
            .is_ok()
        );
    }

    #[test]
    fn debug_request_timestamps_bound_count_bytes_and_release_terminal_ids() {
        let mut timestamps = DebugRequestTimestamps::default();
        for index in 0..MAX_DEBUG_REQUEST_TIMESTAMPS {
            assert!(timestamps.insert(format!("request-{index}"), index as f64));
        }
        assert!(!timestamps.insert("count-overflow".into(), 1.0));
        let (_, retained_before) = timestamps.retained_counts();
        assert_eq!(timestamps.remove("request-7"), Some(7.0));
        let (count_after_terminal, retained_after_terminal) = timestamps.retained_counts();
        assert_eq!(count_after_terminal, MAX_DEBUG_REQUEST_TIMESTAMPS - 1);
        assert_eq!(
            retained_before - retained_after_terminal,
            "request-7".len(),
            "terminal removal refunds the exact owned key bytes"
        );
        assert!(timestamps.insert("replacement".into(), 2.0));

        let mut byte_bounded = DebugRequestTimestamps::default();
        let key_count = MAX_DEBUG_REQUEST_TIMESTAMP_TOTAL_KEY_BYTES
            / MAX_DEBUG_REQUEST_TIMESTAMP_KEY_BYTES;
        for index in 0..key_count {
            let prefix = format!("{index:04}");
            let key = format!(
                "{prefix}{}",
                "x".repeat(MAX_DEBUG_REQUEST_TIMESTAMP_KEY_BYTES - prefix.len())
            );
            assert!(byte_bounded.insert(key, index as f64));
        }
        assert_eq!(
            byte_bounded.retained_counts(),
            (key_count, MAX_DEBUG_REQUEST_TIMESTAMP_TOTAL_KEY_BYTES)
        );
        assert!(!byte_bounded.insert("one-byte-over-total".into(), 0.0));
        assert!(!byte_bounded.insert(
            "x".repeat(MAX_DEBUG_REQUEST_TIMESTAMP_KEY_BYTES + 1),
            0.0
        ));
    }

    #[test]
    fn bounded_lane_inventory_rejects_lineage_count_before_collecting_live_targets() {
        let mut opener = "seed".to_string();
        let mut target_infos = Vec::new();
        for index in 0..MAX_TRACKED_TARGETS_PER_LANE {
            let target_id = format!("descendant-{index}");
            target_infos.push(serde_json::json!({
                "targetId": target_id,
                "type": "page",
                "openerId": opener,
            }));
            opener = format!("descendant-{index}");
        }
        let seed = bounded_lane_seed_targets(["seed"]).expect("one seed is bounded");
        assert_eq!(
            bounded_live_lane_targets(
                &serde_json::json!({"targetInfos": target_infos}),
                seed,
            ),
            Err(BoundedLaneTargetInventoryError::LaneTargetLimit)
        );
    }

    #[test]
    fn bounded_lane_inventory_rejects_host_entries_and_strings_before_id_clones() {
        let too_many = (0..=MAX_TARGET_INVENTORY_ENTRIES_PER_HOST)
            .map(|index| serde_json::json!({"targetId": format!("sibling-{index}"), "type": "page"}))
            .collect::<Vec<_>>();
        assert_eq!(
            bounded_live_lane_targets(
                &serde_json::json!({"targetInfos": too_many}),
                bounded_lane_seed_targets(["seed"]).unwrap(),
            ),
            Err(BoundedLaneTargetInventoryError::HostEntryLimit)
        );

        let oversized_strings = serde_json::json!({
            "targetInfos": [{
                "targetId": "unrelated",
                "type": "page",
                "title": "x".repeat(MAX_TARGET_INVENTORY_STRING_BYTES + 1),
            }]
        });
        assert_eq!(
            bounded_live_lane_targets(
                &oversized_strings,
                bounded_lane_seed_targets(["seed"]).unwrap(),
            ),
            Err(BoundedLaneTargetInventoryError::HostStringByteLimit)
        );
    }

    #[tokio::test]
    async fn bounded_retired_inventory_failure_keeps_exact_tombstone_until_host_cleanup() {
        let (connection, mut requests, server) = generic_recording_fake_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        router
            .state
            .lock()
            .await
            .retired_target_owner
            .insert("retired-target".into(), "retired-lane".into());

        assert!(matches!(
            router.map_lane_inventory_error(
                "retired-lane",
                BoundedLaneTargetInventoryError::HostStringByteLimit,
            ),
            BrowserError::SessionLost { recoverable: false }
        ));
        assert_eq!(
            router
                .state
                .lock()
                .await
                .retired_target_owner
                .get("retired-target")
                .map(String::as_str),
            Some("retired-lane"),
            "capacity rejection must not manufacture an absence proof or drop exact cleanup authority"
        );
        let process_cleanup = tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .expect("capacity failure promptly escalates bounded process cleanup")
            .expect("fake transport remains available");
        assert_eq!(process_cleanup["method"], "Browser.close");

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    async fn router_test_connection() -> (Connection, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind router test websocket");
        let address = listener.local_addr().expect("read router test address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept router test client");
            let _websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete router test websocket handshake");
            std::future::pending::<()>().await;
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect router test websocket");
        (connection, server)
    }

    /// 通用 CDP fake：对每条命令回 `{"id":id,"result":{}}` 并把请求原文推给测试侧
    /// （unbounded channel），供断言方法名/参数与到达顺序。
    async fn generic_recording_fake_connection() -> (
        Connection,
        tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind generic recording fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            while let Some(Ok(message)) = futures_util::StreamExt::next(&mut websocket).await {
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                // 回包须回显 sessionId，否则子 session 命令的回调路由不到。
                let mut response = serde_json::json!({ "id": id, "result": {} });
                if let Some(session_id) = request.get("sessionId") {
                    response["sessionId"] = session_id.clone();
                }
                let _ = request_tx.send(request);
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends generic success");
            }
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect generic recording fake websocket");
        (connection, request_rx, server)
    }

    #[tokio::test]
    async fn task_tab_reservation_rejection_happens_before_create_target() {
        let (connection, mut requests, server) = generic_recording_fake_connection().await;
        let executor = TargetCleanupExecutor::new(connection.clone(), None)
            .expect("bounded cleanup executor starts");
        let error = match create_pending_page_session_owned(
            connection.clone(),
            executor,
            None,
            true,
            Some(TaskTabReservationScope {
                task_resource_key: "task-a".into(),
                lane_id: "lane-a".into(),
                authority: Arc::new(RejectingTaskTabAuthority),
            }),
        )
        .await
        {
            Ok(_) => panic!("reservation rejection must stop target creation"),
            Err(error) => error,
        };
        assert!(matches!(error, BrowserError::Blocked { .. }));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), requests.recv())
                .await
                .is_err(),
            "no CDP command may run after the cross-Host reservation rejects"
        );

        connection.shutdown().await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake transport closes after reservation rollback")
            .expect("fake transport task joins");
    }

    /// **F1 回归**：在防火墙循环 spawn **之前**到达的 `Fetch.requestPaused`（早期
    /// attach 的 target 的首批被拦请求）必须在循环启动后被补处理（continue/fail），
    /// 绝不能因「事件到达时无订阅者」而被丢弃、把请求永久卡死。旧编排（循环内部才
    /// subscribe）下本测试失败：事件在 spawn 前无人订阅即被丢弃。
    #[tokio::test]
    async fn early_paused_request_is_released_once_firewall_loop_starts() {
        let (connection, mut requests, server) = generic_recording_fake_connection().await;

        // 生产编排：订阅先于 attach loop / 防火墙循环。
        let subscriptions = FetchFirewallSubscriptions::subscribe(&connection);
        connection.registry().register_session("S-early", "page");

        // 防火墙循环尚未 spawn 时，一条被拦请求已经到达（早 attach target 的首批请求）。
        connection
            .registry()
            .dispatch_message(
                r#"{"method":"Fetch.requestPaused","sessionId":"S-early","params":{"requestId":"REQ-early","request":{"url":"https://example.com/","method":"GET","headers":{},"initialPriority":"High","referrerPolicy":"no-referrer"},"frameId":"F-early","resourceType":"Document"}}"#,
            )
            .expect("dispatch early paused event");

        let firewall_loop = spawn_fetch_firewall_loop(
            connection.clone(),
            subscriptions,
            crate::firewall::FirewallConfig::default(),
            None,
            crate::firewall::ApprovedDomains::new(),
            Arc::new(crate::firewall::TokioResolver::default()),
            crate::firewall::DnsResolverCache::default(),
        );

        let released = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let request = requests.recv().await.expect("fake server stays alive");
                let method = request["method"].as_str().unwrap_or_default();
                if method == "Fetch.continueRequest" || method == "Fetch.failRequest" {
                    break request;
                }
            }
        })
        .await
        .expect("the buffered paused request must be continued or failed after loop start");
        assert_eq!(released["params"]["requestId"], "REQ-early");
        assert_eq!(released["sessionId"], "S-early");

        firewall_loop.abort();
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    /// **防火墙 watchdog fail-closed**：防火墙任务**非 abort 死亡**（panic 逃出
    /// 循环，如经注入的 EgressApprover trait 对象）时，watchdog 必须把整条 CDP
    /// 连接 fail 掉——后续命令全部 `Closed` 短路，绝不让引擎在「防火墙已死、
    /// 拦截半失效」的状态下继续静默运行。
    #[tokio::test]
    async fn firewall_watchdog_fails_connection_closed_when_firewall_task_dies() {
        let (connection, _requests, server) = generic_recording_fake_connection().await;

        // 模拟防火墙任务死亡：panic 逃出任务体（JoinError::is_panic()==true）。
        let doomed_firewall = tokio::spawn(async {
            panic!("simulated egress firewall death (approver panic)");
        });
        let watchdog = spawn_firewall_watchdog(connection.clone(), doomed_firewall);
        watchdog.await.expect("watchdog itself must not panic");

        // 连接必须已 fail-closed：任何命令都 Closed 短路。
        let error = connection
            .send::<FetchEnableParams>(ROOT_SESSION, &FetchEnableParams::default())
            .await
            .expect_err("commands after firewall death must fail closed");
        assert!(
            matches!(error, TransportError::Closed),
            "expected TransportError::Closed after the watchdog fired, got: {error:?}"
        );

        server.abort();
        let _ = server.await;
    }

    /// **watchdog 不误伤主动关停**：shutdown/Drop 编排对防火墙循环的 `abort()`
    /// 是刻意行为（`JoinError::is_cancelled()`），watchdog 必须无动作——连接保持
    /// 可用（否则每次正常关停都会被 watchdog 抢先 fail 连接，破坏关停命令编排）。
    #[tokio::test]
    async fn firewall_watchdog_ignores_deliberate_abort() {
        let (connection, _requests, server) = generic_recording_fake_connection().await;

        let firewall_loop = tokio::spawn(std::future::pending::<()>());
        let abort_handle = firewall_loop.abort_handle();
        let watchdog = spawn_firewall_watchdog(connection.clone(), firewall_loop);
        abort_handle.abort();
        watchdog.await.expect("watchdog itself must not panic");

        // 连接必须仍可用：主动 abort 不是防火墙死亡。
        connection
            .send::<FetchEnableParams>(ROOT_SESSION, &FetchEnableParams::default())
            .await
            .expect("connection must stay usable after a deliberate abort");

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    // ── F6 白屏回归修：GatePost 无审批通道（托管上下文）→ 放行留痕；硬 Block 仍 fail-closed ──

    /// 测试用 fake DNS：任意 host → 公网 IP（SD-1 DNS 守卫放行，让断言聚焦 GatePost 裁决路径；
    /// 不打真实 DNS，跨平台确定）。
    struct PublicIpResolver;

    #[async_trait::async_trait]
    impl crate::firewall::HostResolver for PublicIpResolver {
        async fn resolve(&self, _host: &str) -> std::io::Result<Vec<std::net::IpAddr>> {
            Ok(vec!["93.184.216.34".parse().unwrap()])
        }
    }

    /// 测试用 fake DNS：任意 host → 私网 IP（SD-1 DNS 守卫必拦）。
    struct PrivateIpResolver;

    #[async_trait::async_trait]
    impl crate::firewall::HostResolver for PrivateIpResolver {
        async fn resolve(&self, _host: &str) -> std::io::Result<Vec<std::net::IpAddr>> {
            Ok(vec!["10.0.0.5".parse().unwrap()])
        }
    }

    struct ResolverCallGuard(Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for ResolverCallGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct BlockingResolver {
        started: Arc<std::sync::atomic::AtomicUsize>,
        dropped: Arc<std::sync::atomic::AtomicUsize>,
        release: CancellationToken,
    }

    #[async_trait::async_trait]
    impl crate::firewall::HostResolver for BlockingResolver {
        async fn resolve(&self, _host: &str) -> std::io::Result<Vec<std::net::IpAddr>> {
            self.started.fetch_add(1, Ordering::SeqCst);
            let _guard = ResolverCallGuard(Arc::clone(&self.dropped));
            self.release.cancelled().await;
            Ok(vec!["93.184.216.34".parse().unwrap()])
        }
    }

    /// 测试用审批者：记录是否被调用 + 返回固定裁决（验「有审批通道时 GatePost 仍路由到审批者」）。
    struct RecordingApprover {
        invoked: Arc<std::sync::atomic::AtomicBool>,
        verdict: crate::firewall::EgressVerdict,
    }

    #[async_trait::async_trait]
    impl crate::firewall::EgressApprover for RecordingApprover {
        async fn approve_egress(
            &self,
            _preview: &crate::firewall::PostPreview,
        ) -> crate::firewall::EgressVerdict {
            self.invoked.store(true, Ordering::SeqCst);
            self.verdict
        }
    }

    struct ApprovalCallGuard {
        active: Arc<std::sync::atomic::AtomicUsize>,
        dropped: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Drop for ApprovalCallGuard {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct BlockingApprover {
        started: Arc<std::sync::atomic::AtomicUsize>,
        active: Arc<std::sync::atomic::AtomicUsize>,
        max_active: Arc<std::sync::atomic::AtomicUsize>,
        dropped: Arc<std::sync::atomic::AtomicUsize>,
        release: CancellationToken,
        verdict: crate::firewall::EgressVerdict,
    }

    #[async_trait::async_trait]
    impl crate::firewall::EgressApprover for BlockingApprover {
        async fn approve_egress(
            &self,
            _preview: &crate::firewall::PostPreview,
        ) -> crate::firewall::EgressVerdict {
            self.started.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            let _guard = ApprovalCallGuard {
                active: Arc::clone(&self.active),
                dropped: Arc::clone(&self.dropped),
            };
            self.release.cancelled().await;
            self.verdict
        }
    }

    fn blocking_approver(
        verdict: crate::firewall::EgressVerdict,
    ) -> (
        Arc<dyn crate::firewall::EgressApprover>,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::atomic::AtomicUsize>,
        CancellationToken,
    ) {
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release = CancellationToken::new();
        (
            Arc::new(BlockingApprover {
                started: Arc::clone(&started),
                active: Arc::clone(&active),
                max_active: Arc::clone(&max_active),
                dropped: Arc::clone(&dropped),
                release: release.clone(),
                verdict,
            }),
            started,
            active,
            max_active,
            dropped,
            release,
        )
    }

    fn dispatch_gatepost_fixture(connection: &Connection, index: usize) {
        connection
            .registry()
            .dispatch_message(
                &serde_json::json!({
                    "method": "Fetch.requestPaused",
                    "sessionId": "S-egress",
                    "params": {
                        "requestId": format!("REQ-burst-{index}"),
                        "request": {
                            "url": format!("https://egress-{index}.example.com/collect"),
                            "method": "POST",
                            "headers": {
                                "Origin": "https://source.example.net",
                                "Content-Type": "application/x-www-form-urlencoded"
                            },
                            "initialPriority": "High",
                            "referrerPolicy": "no-referrer",
                            "hasPostData": true
                        },
                        "frameId": "F-egress",
                        "resourceType": "XHR"
                    }
                })
                .to_string(),
            )
            .expect("dispatch GatePost fixture");
    }

    async fn next_fetch_disposition(
        requests: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
    ) -> serde_json::Value {
        loop {
            let request = requests.recv().await.expect("fake server stays alive");
            if request["method"] == "Fetch.continueRequest"
                || request["method"] == "Fetch.failRequest"
            {
                return request;
            }
        }
    }

    /// 构造一条 `Fetch.requestPaused` 事件 fixture。
    fn paused_event_fixture(
        request_id: &str,
        url: &str,
        method: &str,
        headers: serde_json::Value,
        resource_type: &str,
        has_post_data: bool,
    ) -> EventRequestPaused {
        serde_json::from_value(serde_json::json!({
            "requestId": request_id,
            "request": {
                "url": url,
                "method": method,
                "headers": headers,
                "initialPriority": "High",
                "referrerPolicy": "no-referrer",
                "hasPostData": has_post_data,
            },
            "frameId": "F-egress",
            "resourceType": resource_type,
        }))
        .expect("valid EventRequestPaused fixture")
    }

    /// 把一条被拦请求喂给 `handle_paused_request`，返回它在 CDP wire 上的最终处置
    /// （`Fetch.continueRequest` 或 `Fetch.failRequest` 的请求原文）。
    async fn drive_paused_request(
        config: crate::firewall::FirewallConfig,
        approver: Option<Arc<dyn crate::firewall::EgressApprover>>,
        paused: EventRequestPaused,
        resolver: &dyn crate::firewall::HostResolver,
    ) -> serde_json::Value {
        let (connection, mut requests, server) = generic_recording_fake_connection().await;
        connection.registry().register_session("S-egress", "page");
        let cancel = CancellationToken::new();
        let approval = handle_paused_request(
            &connection,
            &config,
            approver.as_ref(),
            &crate::firewall::ApprovedDomains::new(),
            "S-egress",
            paused,
            resolver,
            &crate::firewall::DnsResolverCache::default(),
            &cancel,
        )
        .await;
        if let Some(approval) = approval {
            resolve_firewall_approval(
                approval,
                &cancel,
                crate::firewall::EGRESS_APPROVAL_TIMEOUT,
            )
            .await;
        }
        let released = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let request = requests.recv().await.expect("fake server stays alive");
                let method = request["method"].as_str().unwrap_or_default();
                if method == "Fetch.continueRequest" || method == "Fetch.failRequest" {
                    break request;
                }
            }
        })
        .await
        .expect("the paused request must be continued or failed");
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
        released
    }

    /// **F6 白屏回归修**：无审批通道（托管上下文）时，域 allowlist 外的**跨站子资源**命中 GatePost
    /// 必须 `Fetch.continueRequest` 放行（+ warn 留痕），**绝不** `failRequest` 白屏。
    /// （旧实现无通道恒 fail-closed → 本测试对修复前 HEAD 失败。）
    #[tokio::test]
    async fn no_approver_gated_off_allowlist_subresource_is_continued_with_audit() {
        let config = crate::firewall::FirewallConfig {
            allow_etld1: vec!["stored-secret.com".to_string()],
            ..Default::default()
        };
        let paused = paused_event_fixture(
            "REQ-allowlist-gate",
            "https://tracker.io/collect",
            "GET",
            serde_json::json!({"Referer": "https://news.example.com/story"}),
            "XHR",
            false,
        );
        let released = drive_paused_request(config, None, paused, &PublicIpResolver).await;
        assert_eq!(
            released["method"], "Fetch.continueRequest",
            "no-approver GatePost (domain allowlist) must allow-and-audit, not fail: {released}"
        );
        assert_eq!(released["params"]["requestId"], "REQ-allowlist-gate");
        assert_eq!(released["sessionId"], "S-egress");
    }

    /// **F6 白屏回归修**：无审批通道时，跨域 POST-body 门控命中的 GatePost 同样放行留痕
    /// （E5 pre-approval 姿态：检测+留痕，审批接线前放行），绝不 failRequest。
    #[tokio::test]
    async fn no_approver_gated_cross_origin_post_is_continued_with_audit() {
        let config = crate::firewall::FirewallConfig::default(); // 空 allowlist；POST 门控开
        let paused = paused_event_fixture(
            "REQ-post-gate",
            "https://evil.com/collect",
            "POST",
            serde_json::json!({
                "Origin": "https://x.com",
                "Content-Type": "application/x-www-form-urlencoded",
            }),
            "XHR",
            true,
        );
        let released = drive_paused_request(config, None, paused, &PublicIpResolver).await;
        assert_eq!(
            released["method"], "Fetch.continueRequest",
            "no-approver GatePost (cross-origin POST gate) must allow-and-audit, not fail: {released}"
        );
        assert_eq!(released["params"]["requestId"], "REQ-post-gate");
    }

    /// **硬 Block 不受 allow-and-audit 影响①**：SSRF IP 封禁（IP 字面量目标）即便无审批通道
    /// 仍 `Fetch.failRequest`（fail-closed；访问元数据/内网无「批准」语义）。
    #[tokio::test]
    async fn no_approver_ssrf_ip_literal_block_still_fails_closed() {
        let config = crate::firewall::FirewallConfig::default();
        let paused = paused_event_fixture(
            "REQ-ssrf-literal",
            "http://169.254.169.254/latest/meta-data/",
            "GET",
            serde_json::json!({"Referer": "https://x.com/"}),
            "XHR",
            false,
        );
        let released = drive_paused_request(config, None, paused, &PublicIpResolver).await;
        assert_eq!(
            released["method"], "Fetch.failRequest",
            "SSRF IP block must stay fail-closed even with no approver: {released}"
        );
        assert_eq!(released["params"]["requestId"], "REQ-ssrf-literal");
    }

    /// **硬 Block 不受 allow-and-audit 影响②**：SD-1 DNS→私网 IP 守卫即便无审批通道仍 failRequest。
    #[tokio::test]
    async fn no_approver_dns_ssrf_block_still_fails_closed() {
        let config = crate::firewall::FirewallConfig::default();
        let paused = paused_event_fixture(
            "REQ-ssrf-dns",
            "https://rebind.attacker.example/x",
            "GET",
            serde_json::json!({"Referer": "https://x.com/"}),
            "XHR",
            false,
        );
        let released = drive_paused_request(config, None, paused, &PrivateIpResolver).await;
        assert_eq!(
            released["method"], "Fetch.failRequest",
            "DNS→private-IP SSRF guard must stay fail-closed even with no approver: {released}"
        );
        assert_eq!(released["params"]["requestId"], "REQ-ssrf-dns");
    }

    /// **硬 Block 不受 allow-and-audit 影响③**：deny_etld1 黑名单即便无审批通道、即便**同站**请求，
    /// 仍 failRequest（显式封禁名单优先一切豁免/降级）。
    #[tokio::test]
    async fn no_approver_deny_etld1_block_still_fails_closed() {
        let config = crate::firewall::FirewallConfig {
            deny_etld1: vec!["evil.com".to_string()],
            ..Default::default()
        };
        let paused = paused_event_fixture(
            "REQ-deny",
            "https://sub.evil.com/asset.js",
            "GET",
            serde_json::json!({"Referer": "https://evil.com/page"}), // 同站也拦
            "XHR",
            false,
        );
        let released = drive_paused_request(config, None, paused, &PublicIpResolver).await;
        assert_eq!(
            released["method"], "Fetch.failRequest",
            "deny_etld1 must stay a hard fail-closed block even with no approver: {released}"
        );
        assert_eq!(released["params"]["requestId"], "REQ-deny");
    }

    /// **standalone 路径不变**：有审批通道时 GatePost 仍路由到 `EgressApprover`（悬挂等裁决），
    /// 拒 → failRequest、批 → continueRequest——allow-and-audit **只**作用于无通道分支。
    #[tokio::test]
    async fn approver_present_gatepost_still_routes_to_approver() {
        for (verdict, expected_method) in [
            (crate::firewall::EgressVerdict::Fail, "Fetch.failRequest"),
            (crate::firewall::EgressVerdict::Continue, "Fetch.continueRequest"),
        ] {
            let invoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let approver: Arc<dyn crate::firewall::EgressApprover> = Arc::new(RecordingApprover {
                invoked: Arc::clone(&invoked),
                verdict,
            });
            let config = crate::firewall::FirewallConfig {
                allow_etld1: vec!["stored-secret.com".to_string()],
                ..Default::default()
            };
            let paused = paused_event_fixture(
                "REQ-approver",
                "https://tracker.io/collect",
                "GET",
                serde_json::json!({"Referer": "https://news.example.com/story"}),
                "XHR",
                false,
            );
            let released =
                drive_paused_request(config, Some(approver), paused, &PublicIpResolver).await;
            assert!(
                invoked.load(Ordering::SeqCst),
                "a wired approver must be consulted for GatePost (verdict {verdict:?})"
            );
            assert_eq!(
                released["method"], expected_method,
                "approver verdict {verdict:?} must drive the wire disposition: {released}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_gatepost_burst_has_fixed_approval_concurrency_and_bounded_overflow() {
        const REQUESTS: usize = 24;
        const APPROVAL_WORKERS: usize = 2;
        const APPROVAL_QUEUE: usize = 3;

        let (connection, mut wire, server) = generic_recording_fake_connection().await;
        let subscriptions = FetchFirewallSubscriptions::subscribe(&connection);
        connection.registry().register_session("S-egress", "page");
        let (approver, _started, active, max_active, _dropped, release) =
            blocking_approver(crate::firewall::EgressVerdict::Continue);
        let runtime = spawn_fetch_firewall_loop_with_limits(
            connection.clone(),
            subscriptions,
            crate::firewall::FirewallConfig::default(),
            Some(approver),
            crate::firewall::ApprovedDomains::new(),
            Arc::new(PublicIpResolver),
            crate::firewall::DnsResolverCache::default(),
            FirewallExecutorLimits {
                request_workers: 4,
                request_queue_capacity: 64,
                approval_workers: APPROVAL_WORKERS,
                approval_queue_capacity: APPROVAL_QUEUE,
                approval_timeout: Duration::from_secs(5),
                shutdown_join_timeout: Duration::from_millis(500),
            },
        );

        for index in 0..REQUESTS {
            dispatch_gatepost_fixture(&connection, index);
        }

        let first_overflow = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let disposition = next_fetch_disposition(&mut wire).await;
                if disposition["method"] == "Fetch.failRequest" {
                    break disposition;
                }
            }
        })
        .await
        .expect("approval queue saturation rejects promptly");
        assert_eq!(first_overflow["method"], "Fetch.failRequest");
        assert!(
            max_active.load(Ordering::SeqCst) <= APPROVAL_WORKERS,
            "approval concurrency must never exceed the fixed worker count"
        );

        release.cancel();
        let mut disposed = HashSet::from([
            first_overflow["params"]["requestId"]
                .as_str()
                .expect("overflow request id")
                .to_string(),
        ]);
        let mut failed = 1usize;
        tokio::time::timeout(Duration::from_secs(5), async {
            while disposed.len() < REQUESTS {
                let disposition = next_fetch_disposition(&mut wire).await;
                if disposition["method"] == "Fetch.failRequest" {
                    failed += 1;
                }
                assert!(
                    disposed.insert(
                        disposition["params"]["requestId"]
                            .as_str()
                            .expect("disposition request id")
                            .to_string(),
                    ),
                    "each paused request must receive exactly one disposition"
                );
            }
        })
        .await
        .expect("all admitted and rejected requests finish");
        assert!(failed > 0, "the bounded approval queue must reject overflow");
        assert_eq!(active.load(Ordering::SeqCst), 0);

        runtime.shutdown().await;
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_approval_timeout_fails_closed_and_drops_the_approval_future() {
        let (connection, mut wire, server) = generic_recording_fake_connection().await;
        let subscriptions = FetchFirewallSubscriptions::subscribe(&connection);
        connection.registry().register_session("S-egress", "page");
        let (approver, started, active, _max_active, dropped, _release) =
            blocking_approver(crate::firewall::EgressVerdict::Continue);
        let runtime = spawn_fetch_firewall_loop_with_limits(
            connection.clone(),
            subscriptions,
            crate::firewall::FirewallConfig::default(),
            Some(approver),
            crate::firewall::ApprovedDomains::new(),
            Arc::new(PublicIpResolver),
            crate::firewall::DnsResolverCache::default(),
            FirewallExecutorLimits {
                request_workers: 1,
                request_queue_capacity: 4,
                approval_workers: 1,
                approval_queue_capacity: 1,
                approval_timeout: Duration::from_millis(30),
                shutdown_join_timeout: Duration::from_millis(500),
            },
        );
        dispatch_gatepost_fixture(&connection, 0);

        let disposition = tokio::time::timeout(
            Duration::from_secs(2),
            next_fetch_disposition(&mut wire),
        )
        .await
        .expect("slow approval reaches its bounded timeout");
        assert_eq!(disposition["method"], "Fetch.failRequest");
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(
            dropped.load(Ordering::SeqCst),
            1,
            "timeout must drop, not detach, the approval future"
        );

        runtime.shutdown().await;
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn firewall_shutdown_cancels_and_joins_a_pending_approval() {
        let (connection, _wire, server) = generic_recording_fake_connection().await;
        let subscriptions = FetchFirewallSubscriptions::subscribe(&connection);
        connection.registry().register_session("S-egress", "page");
        let (approver, started, active, _max_active, dropped, _release) =
            blocking_approver(crate::firewall::EgressVerdict::Continue);
        let runtime = spawn_fetch_firewall_loop_with_limits(
            connection.clone(),
            subscriptions,
            crate::firewall::FirewallConfig::default(),
            Some(approver),
            crate::firewall::ApprovedDomains::new(),
            Arc::new(PublicIpResolver),
            crate::firewall::DnsResolverCache::default(),
            FirewallExecutorLimits {
                request_workers: 1,
                request_queue_capacity: 4,
                approval_workers: 1,
                approval_queue_capacity: 1,
                approval_timeout: Duration::from_secs(60),
                shutdown_join_timeout: Duration::from_millis(500),
            },
        );
        dispatch_gatepost_fixture(&connection, 0);
        tokio::time::timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("approval starts");

        tokio::time::timeout(Duration::from_secs(1), runtime.shutdown())
            .await
            .expect("firewall shutdown is bounded");
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(
            dropped.load(Ordering::SeqCst),
            1,
            "Host cancellation must drop the pending approval future before returning"
        );

        connection
            .send::<FetchEnableParams>(ROOT_SESSION, &FetchEnableParams::default())
            .await
            .expect("deliberate firewall cancellation must not trip the death watchdog");
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn saturated_firewall_request_queue_rejects_and_closes_fail_closed() {
        let (connection, mut wire, server) = generic_recording_fake_connection().await;
        let subscriptions = FetchFirewallSubscriptions::subscribe(&connection);
        connection.registry().register_session("S-egress", "page");
        let resolver_started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let resolver_dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = spawn_fetch_firewall_loop_with_limits(
            connection.clone(),
            subscriptions,
            crate::firewall::FirewallConfig::default(),
            None,
            crate::firewall::ApprovedDomains::new(),
            Arc::new(BlockingResolver {
                started: Arc::clone(&resolver_started),
                dropped: Arc::clone(&resolver_dropped),
                release: CancellationToken::new(),
            }),
            crate::firewall::DnsResolverCache::default(),
            FirewallExecutorLimits {
                request_workers: 1,
                request_queue_capacity: 1,
                approval_workers: 1,
                approval_queue_capacity: 1,
                approval_timeout: Duration::from_secs(5),
                shutdown_join_timeout: Duration::from_millis(500),
            },
        );

        dispatch_gatepost_fixture(&connection, 0);
        tokio::time::timeout(Duration::from_secs(2), async {
            while resolver_started.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the sole request worker is occupied by slow DNS");
        dispatch_gatepost_fixture(&connection, 1);
        dispatch_gatepost_fixture(&connection, 2);

        let rejected = tokio::time::timeout(
            Duration::from_secs(2),
            next_fetch_disposition(&mut wire),
        )
        .await
        .expect("queue overflow is rejected promptly");
        assert_eq!(rejected["method"], "Fetch.failRequest");
        assert_eq!(rejected["params"]["requestId"], "REQ-burst-2");
        tokio::time::timeout(Duration::from_secs(2), async {
            while !connection.registry().is_connection_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("overflow poisons the connection after dispatching the rejection");
        let error = connection
            .send::<FetchEnableParams>(ROOT_SESSION, &FetchEnableParams::default())
            .await
            .expect_err("an overflowed reliable firewall queue poisons the Host connection");
        assert!(matches!(error, TransportError::Closed));

        runtime.shutdown().await;
        assert_eq!(
            resolver_dropped.load(Ordering::SeqCst),
            1,
            "overflow teardown must drop the occupied resolver future"
        );
        server.abort();
        let _ = server.await;
    }

    /// `downloadWillBegin` 突发，**每一条**被阻断的下载都必须发出 cancelDownload。
    /// 旧实现（lossy `subscribe` + `Lagged → continue`）在突发下静默丢事件，
    /// 丢掉的 .exe 下载不再被取消——红线被时序绕过。
    #[tokio::test]
    async fn executable_download_burst_never_drops_red_line_cancels() {
        const BURST: usize = crate::session::RELIABLE_EVENT_CAPACITY;

        let (connection, mut requests, server) = generic_recording_fake_connection().await;
        let download_loop = spawn_download_loop(connection.clone(), None);

        // 紧凑同步派发（无 await 点）：当前线程 runtime 下循环任务无机会消费，
        // lossy broadcast 必然溢出丢事件；有界可靠通道在其硬容量内全量缓存。
        for index in 0..BURST {
            connection
                .registry()
                .dispatch_message(&format!(
                    r#"{{"method":"Browser.downloadWillBegin","params":{{"frameId":"F1","guid":"guid-{index}","url":"https://example.com/evil.exe","suggestedFilename":"evil-{index}.exe"}}}}"#
                ))
                .expect("dispatch downloadWillBegin");
        }

        let mut cancelled = HashSet::new();
        tokio::time::timeout(Duration::from_secs(30), async {
            while cancelled.len() < BURST {
                let request = requests.recv().await.expect("fake server stays alive");
                if request["method"] == "Browser.cancelDownload" {
                    cancelled.insert(
                        request["params"]["guid"].as_str().unwrap().to_string(),
                    );
                }
            }
        })
        .await
        .expect("every blocked executable download must be cancelled");
        for index in 0..BURST {
            assert!(
                cancelled.contains(&format!("guid-{index}")),
                "cancelDownload for guid-{index} must not be dropped under burst"
            );
        }

        download_loop.abort();
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn executable_download_burst_above_reliable_bound_poison_closes_fail_closed() {
        let (connection, _requests, server) = generic_recording_fake_connection().await;
        let download_loop = spawn_download_loop(connection.clone(), None);

        for index in 0..crate::session::RELIABLE_EVENT_CAPACITY {
            connection
                .registry()
                .dispatch_message(&format!(
                    r#"{{"method":"Browser.downloadWillBegin","params":{{"frameId":"F1","guid":"guid-{index}","url":"https://example.com/evil.exe","suggestedFilename":"evil-{index}.exe"}}}}"#
                ))
                .expect("events through the exact reliable bound are admitted");
        }
        let overflow = connection.registry().dispatch_message(
            r#"{"method":"Browser.downloadWillBegin","params":{"frameId":"F1","guid":"guid-overflow","url":"https://example.com/evil.exe","suggestedFilename":"evil-overflow.exe"}}"#,
        );
        assert!(overflow.is_err(), "the first event above the hard bound is rejected");
        assert!(
            connection.registry().is_connection_closed(),
            "reliable red-line overflow poisons the CDP connection instead of dropping an executable download silently"
        );

        download_loop.abort();
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    /// **F46 回归**：阻断日志的 url scheme 提取绝不 panic——空 URL、无冒号、冒号
    /// 过深、冒号前多字节字符统统安全（旧裸切片对空串越界 panic，杀死下载循环）。
    #[test]
    fn blocked_download_url_scheme_never_panics() {
        assert_eq!(blocked_download_url_scheme(""), "[no-scheme]");
        assert_eq!(blocked_download_url_scheme("no-colon-here"), "[no-scheme]");
        assert_eq!(blocked_download_url_scheme("https://x/evil.exe"), "https:");
        assert_eq!(blocked_download_url_scheme("data:app/x;base64,TVo="), "data:");
        assert_eq!(blocked_download_url_scheme("blob:https://x"), "blob:");
        // 冒号正好在第 10 字节（旧实现的钳制边界）。
        assert_eq!(blocked_download_url_scheme("abcdefghij:x"), "abcdefghij:");
        // 冒号 >10 字节：不切片（旧实现会切进 scheme 中部）。
        assert_eq!(blocked_download_url_scheme("verylongscheme:x"), "[odd-scheme]");
        // 冒号前多字节字符（旧实现可能切在 char 边界内 panic）。
        assert_eq!(blocked_download_url_scheme("приложение:x"), "[odd-scheme]");
        // 多字节但冒号 ≤10 字节：切片边界紧跟单字节 ':' 之后，恒安全。
        assert_eq!(blocked_download_url_scheme("网页:x"), "网页:");
    }

    #[test]
    fn download_progress_numeric_conversion_fails_closed() {
        assert_eq!(download_progress_bytes(0.0), Some(0));
        assert_eq!(download_progress_bytes(1.01), Some(2));
        assert_eq!(download_progress_bytes(-0.01), None);
        assert_eq!(download_progress_bytes(f64::NAN), None);
        assert_eq!(download_progress_bytes(f64::INFINITY), None);
        assert_eq!(download_progress_bytes(f64::NEG_INFINITY), None);
    }

    /// 清理/下载路由测试用 fake：`Target.closeTarget` → `success:true`；
    /// `Target.getTargets` → 依次弹 `inventories`（耗尽后恒空）；`Page.getFrameTree` →
    /// `frame_tree`（`None` → `{}`）；其它 → `{}`。所有回包回显 sessionId；每条请求
    /// 推给测试侧供断言。
    async fn cleanup_routing_fake_connection(
        inventories: Vec<Vec<serde_json::Value>>,
        frame_tree: Option<serde_json::Value>,
    ) -> (
        Connection,
        tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cleanup-routing fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut inventories = std::collections::VecDeque::from(inventories);
            while let Some(Ok(message)) = futures_util::StreamExt::next(&mut websocket).await {
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                let method = request["method"].as_str().unwrap_or_default();
                let result = match method {
                    "Target.closeTarget" => serde_json::json!({ "success": true }),
                    "Target.getTargets" => serde_json::json!({
                        "targetInfos": inventories.pop_front().unwrap_or_default()
                    }),
                    "Page.getFrameTree" => frame_tree
                        .clone()
                        .map(|tree| serde_json::json!({ "frameTree": tree }))
                        .unwrap_or_else(|| serde_json::json!({})),
                    _ => serde_json::json!({}),
                };
                let mut response = serde_json::json!({ "id": id, "result": result });
                if let Some(session_id) = request.get("sessionId") {
                    response["sessionId"] = session_id.clone();
                }
                let _ = request_tx.send(request);
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends response");
            }
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect cleanup-routing fake websocket");
        (connection, request_rx, server)
    }

    /// **F20 回归**：显式 shutdown 已推进到 unregister（owned targets 已成 retired
    /// tombstones）但 finalize 失败后，调用方 drop 了 backend——Drop/hand_off 兜底
    /// 路径的 `finish` 必须**重驱** finalize，关掉仍存活的 retired 目标；旧实现
    /// `if unregistered && …` 在「本次调用没做 unregister」时直接跳过，泄漏目标。
    #[tokio::test]
    async fn drop_path_finalizes_a_previously_unregistered_lane_with_live_retired_targets() {
        let (connection, mut requests, server) = cleanup_routing_fake_connection(
            vec![vec![serde_json::json!({ "targetId": "leaked-popup", "type": "page" })]],
            None,
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();

        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_target = Arc::new(AsyncMutex::new("old-target".to_string()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        let lane_closing = Arc::new(AtomicBool::new(false));
        let registration = router
            .register_lane(
                "retired-lane".into(),
                &tabs,
                &active_target,
                &active_frame,
                Arc::clone(&lane_closing),
                None,
            )
            .await
            .expect("lane registers");
        assert!(router.claim_target("retired-lane", "old-target").await);
        assert!(router.claim_target("retired-lane", "leaked-popup").await);
        // 显式 shutdown 已 unregister（tombstones 建立）但 finalize 失败的残局形态。
        assert!(
            router
                .unregister_lane_if_current("retired-lane", registration)
                .await
        );

        let cleanup = LaneCleanupAuthority::new(
            connection.clone(),
            Arc::clone(&router.cleanup_executor),
            Arc::clone(&router),
            "retired-lane".into(),
            lane_closing,
            Arc::clone(&tabs),
            "old-target".into(),
            "old-session".into(),
            "old-frame".into(),
            None,
            None,
        );
        cleanup.set_registration(registration);

        tokio::time::timeout(Duration::from_secs(20), Arc::clone(&cleanup).finish())
            .await
            .expect("drop-path finish completes");
        assert_eq!(cleanup.state.load(Ordering::Acquire), 2);

        let mut closed_targets = Vec::new();
        while let Ok(request) = requests.try_recv() {
            if request["method"] == "Target.closeTarget" {
                closed_targets
                    .push(request["params"]["targetId"].as_str().unwrap().to_string());
            }
        }
        assert!(closed_targets.contains(&"old-target".to_string()));
        assert!(
            closed_targets.contains(&"leaked-popup".to_string()),
            "the still-live retired popup must be closed by the Drop/hand_off fallback: {closed_targets:?}"
        );

        router_loop.abort();
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    /// **F21 回归**：iframe 子帧发起的下载（downloadWillBegin 带子帧 frameId，
    /// frame_owner/ownership 恒查不到）必须经 frame-tree 解析归属其 lane 并被
    /// finish_download 路由到 lane 工作区；旧实现直接检疫在 Host staging，
    /// act_download 永远等不到文件。
    #[tokio::test]
    async fn subframe_download_routes_to_its_owning_lane() {
        let temp = tempfile::tempdir().expect("create download routing root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane downloads dir");

        let (connection, _requests, server) = cleanup_routing_fake_connection(
            Vec::new(),
            Some(serde_json::json!({
                "frame": { "id": "tab-1" },
                "childFrames": [{ "frame": { "id": "sub-frame-9" } }]
            })),
        )
        .await;
        connection.registry().register_session("session-tab-1", "page");

        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );
        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        tabs.lock().await.insert(
            "tab-1".to_string(),
            test_tab_record(&connection, "tab-1", "session-tab-1"),
        );
        let active_target = Arc::new(AsyncMutex::new("tab-1".to_string()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane(
                "lane-a".into(),
                &tabs,
                &active_target,
                &active_frame,
                Arc::new(AtomicBool::new(false)),
                Some(lane_dir.to_string_lossy().into_owned()),
            )
            .await
            .expect("lane registers");
        assert!(router.claim_target("lane-a", "tab-1").await);
        router.claim_frame("lane-a", "tab-1").await;

        // 子帧下载：frameId 是 iframe 的，不在 frame_owner（主帧表）/ownership（target 表）。
        assert!(
            router
                .begin_download("sub-frame-9", "guid-sub", "report.pdf")
                .await
        );

        let staged = staging.join("guid-sub");
        std::fs::write(&staged, b"pdf-bytes").expect("stage downloaded file");
        assert!(
            router.finish_download("guid-sub", &staged).await,
            "a subframe-initiated download must be routed to its owning lane"
        );
        assert_eq!(
            std::fs::read(lane_dir.join("report.pdf")).expect("routed file exists"),
            b"pdf-bytes"
        );

        // 真正不属于任何 lane 的帧拒绝 admission；若取消竞态下文件仍落到
        // Host staging，terminal reconciliation 精确删除而不误配 lane。
        assert!(
            !router
                .begin_download("frame-of-no-lane", "guid-alien", "alien.bin")
                .await
        );
        let alien = staging.join("guid-alien");
        std::fs::write(&alien, b"alien").expect("stage alien file");
        assert!(
            !router.finish_download("guid-alien", &alien).await,
            "an unowned frame cannot acquire a lane route"
        );
        router.cleanup_staged_download("guid-alien", Some(&alien));
        assert!(!alien.exists(), "unowned staging artifact is removed");

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn download_terminal_ttl_and_shutdown_reconciliation_clear_state_and_staging() {
        let temp = tempfile::tempdir().expect("create download reconciliation root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );

        let insert_route = |guid: &str, created_at: std::time::Instant| {
            router
                .download_ledger
                .downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    guid.into(),
                    PendingDownload {
                        lane_id: "lane-a".into(),
                        download_dir: lane_dir.to_string_lossy().into_owned(),
                        suggested_filename: format!("{guid}.pdf"),
                        created_at,
                        cancel_requested_at: None,
                        reservation: Arc::new(TestTaskDownloadReservation),
                    },
                );
            std::fs::write(staging.join(guid), b"partial").expect("stage route artifact");
        };

        insert_route("guid-cancel", std::time::Instant::now());
        router.cancel_pending_download("guid-cancel");
        assert!(!staging.join("guid-cancel").exists());

        assert!(router.quarantine_rejected_download("guid-rejected"));
        std::fs::write(staging.join("guid-rejected"), b"late-after-cancel")
            .expect("late rejected artifact lands");
        router.cancel_pending_download("guid-rejected");
        assert!(!staging.join("guid-rejected").exists());

        insert_route(
            "guid-expired",
            std::time::Instant::now() - DOWNLOAD_ROUTE_TTL - Duration::from_secs(1),
        );
        assert_eq!(router.expire_pending_downloads(), vec!["guid-expired"]);
        assert!(staging.join("guid-expired").exists());
        assert_eq!(router.pending_download_count(), 1);

        insert_route("guid-lagged", std::time::Instant::now());
        insert_route("guid-shutdown", std::time::Instant::now());
        let mut reconciled = router.finalize_downloads_after_host_stop();
        reconciled.sort();
        assert_eq!(
            reconciled,
            vec!["guid-expired", "guid-lagged", "guid-shutdown"]
        );
        assert_eq!(router.pending_download_count(), 0);
        assert!(!staging.join("guid-lagged").exists());
        assert!(!staging.join("guid-shutdown").exists());

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn failed_staging_delete_retains_exact_bounded_retry_authority() {
        let temp = tempfile::tempdir().expect("create download retry root");
        let staging = temp.path().join("host-staging");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );

        // A directory at the exact file path makes remove_file fail on every
        // platform without relying on Windows-only sharing semantics.
        let blocked_path = staging.join("guid-retry");
        std::fs::create_dir(&blocked_path).expect("create undeletable-as-file fixture");
        router.cleanup_staged_download("guid-retry", Some(&blocked_path));
        assert_eq!(
            router
                .download_ledger
                .download_cleanup_retries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1,
            "failed deletion retains one exact retry path"
        );
        std::fs::remove_dir(&blocked_path).expect("release cleanup fixture");
        assert_eq!(router.retry_staging_cleanup(), 0);

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn full_active_and_rejected_download_inventory_retains_every_cleanup_path() {
        let temp = tempfile::tempdir().expect("create full download retry root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );

        let mut expected_paths = HashSet::new();
        let make_failed_paths = |guid: &str, expected_paths: &mut HashSet<PathBuf>| {
            for name in [
                guid.to_string(),
                format!("{guid}.crdownload"),
                format!("{guid}.tmp"),
            ] {
                let path = staging.join(name);
                std::fs::create_dir(&path)
                    .expect("directory fixture makes remove_file fail on every platform");
                expected_paths.insert(path);
            }
        };

        {
            let mut downloads = router
                .download_ledger
                .downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for index in 0..MAX_PENDING_DOWNLOADS_PER_HOST {
                let guid = format!("active-{index:03}");
                make_failed_paths(&guid, &mut expected_paths);
                downloads.insert(
                    guid.clone(),
                    PendingDownload {
                        lane_id: "lane-a".into(),
                        download_dir: lane_dir.to_string_lossy().into_owned(),
                        suggested_filename: format!("{guid}.pdf"),
                        created_at: std::time::Instant::now(),
                        cancel_requested_at: None,
                        reservation: Arc::new(TestTaskDownloadReservation),
                    },
                );
            }
        }
        {
            let mut rejected = router
                .download_ledger
                .rejected_downloads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for index in 0..MAX_QUARANTINED_DOWNLOADS_PER_HOST {
                let guid = format!("rejected-{index:03}");
                make_failed_paths(&guid, &mut expected_paths);
                rejected.insert(guid, std::time::Instant::now());
            }
        }

        let reconciled = router.finalize_downloads_after_host_stop();
        assert_eq!(
            reconciled.len(),
            MAX_PENDING_DOWNLOADS_PER_HOST + MAX_QUARANTINED_DOWNLOADS_PER_HOST
        );
        let retained = router
            .download_ledger
            .download_cleanup_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(retained.len(), MAX_DOWNLOAD_CLEANUP_RETRIES);
        assert_eq!(retained, expected_paths, "no exact cleanup path may be lost");
        assert!(
            !router
                .download_ledger
                .download_cleanup_poisoned
                .load(Ordering::Acquire),
            "the complete admitted 64+64 inventory must fit its exact retry authority"
        );
        assert!(!router.download_ledger.directory_cleanup_pending());

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn staging_scan_cursor_eventually_reaches_old_tail_past_undeletable_prefix() {
        let temp = tempfile::tempdir().expect("create staging scan root");
        let staging = temp.path().join("host-staging");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );

        // More than one scan batch of directories models a permanently
        // ineligible/undeletable prefix without platform-specific file locks.
        for index in 0..MAX_DOWNLOAD_STAGING_SCAN_ENTRIES {
            std::fs::create_dir(staging.join(format!("0000-blocked-{index:04}")))
                .expect("create blocked prefix entry");
        }
        let old_tail = staging.join("ffff-tail-artifact");
        std::fs::write(&old_tail, b"stale").expect("create old tail artifact");
        let future = std::time::SystemTime::now()
            .checked_add(DOWNLOAD_ROUTE_TTL + Duration::from_secs(1))
            .expect("test clock remains representable");

        // A retained ReadDir cursor advances rather than restarting at the
        // same 512 entries. Allow a few passes because directory enumeration
        // order is platform-defined; convergence, not first-pass order, is the
        // contract.
        for _ in 0..4 {
            router.download_ledger.sweep_stale_staging_files_at(future);
            if !old_tail.exists() {
                break;
            }
        }
        assert!(
            !old_tail.exists(),
            "an old artifact beyond a full blocked batch must not starve forever"
        );

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn staging_cleanup_rejects_path_components_and_traversal() {
        assert_eq!(safe_download_guid_component("guid-123"), Some("guid-123"));
        assert_eq!(safe_download_guid_component(""), None);
        assert_eq!(safe_download_guid_component("../escape"), None);
        assert_eq!(safe_download_guid_component("nested/file"), None);
        assert_eq!(safe_download_guid_component("..\\escape"), None);
        assert!(is_chromium_download_guid_name(
            "550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(is_chromium_download_guid_name(
            "550e8400-e29b-41d4-a716-446655440000.crdownload"
        ));
        assert!(!is_chromium_download_guid_name("report.pdf"));
        assert!(!is_chromium_download_guid_name("guid-rejected"));
    }

    /// Byte/lifecycle-recording download reservation for two-phase publish
    /// assertions (the plain [`TestTaskDownloadReservation`] ignores bytes).
    struct RecordingTaskDownloadReservation {
        prepared_bytes: Mutex<Option<u64>>,
        finalized: AtomicBool,
    }

    impl RecordingTaskDownloadReservation {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                prepared_bytes: Mutex::new(None),
                finalized: AtomicBool::new(false),
            })
        }

        fn prepared(&self) -> Option<u64> {
            *self
                .prepared_bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        fn is_finalized(&self) -> bool {
            self.finalized.load(Ordering::Acquire)
        }
    }

    impl TaskDownloadReservation for RecordingTaskDownloadReservation {
        fn update_progress(
            &self,
            _received_bytes: u64,
            _total_bytes: Option<u64>,
        ) -> Result<(), BrowserError> {
            Ok(())
        }

        fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserError> {
            self.prepared_bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .replace(actual_bytes);
            Ok(())
        }

        fn finalize_complete(&self) {
            self.finalized.store(true, Ordering::Release);
        }
    }

    fn ledger_route(
        lane_dir: &std::path::Path,
        filename: &str,
        reservation: Arc<dyn TaskDownloadReservation>,
    ) -> PendingDownload {
        PendingDownload {
            lane_id: "lane-a".into(),
            download_dir: lane_dir.to_string_lossy().into_owned(),
            suggested_filename: filename.into(),
            created_at: std::time::Instant::now(),
            cancel_requested_at: None,
            reservation,
        }
    }

    /// P2 回归：finish_download 走两阶段补偿事务——prepare 用**实际落盘字节数**
    /// 记账(不信 CDP total)、经唯一临时文件原子发布、不覆盖既有产物、发布成功后
    /// 才 finalize 永久入账。
    #[tokio::test]
    async fn finish_download_two_phase_charges_actual_size_and_never_clobbers() {
        let temp = tempfile::tempdir().expect("create publish test root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");
        let ledger = HostDownloadLedger::new(Some(staging.clone()));

        // A pre-existing destination must never be overwritten.
        std::fs::write(lane_dir.join("report.pdf"), b"pre-existing")
            .expect("seed existing output");
        let reservation = RecordingTaskDownloadReservation::new();
        ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "guid-pub".into(),
                ledger_route(&lane_dir, "report.pdf", reservation.clone()),
            );
        let staged = staging.join("guid-pub");
        std::fs::write(&staged, b"7-bytes").expect("stage artifact");

        assert!(ledger.finish_download("guid-pub", &staged));
        assert_eq!(
            reservation.prepared(),
            Some(7),
            "the charge must use the actual on-disk size"
        );
        assert!(reservation.is_finalized());
        assert_eq!(
            std::fs::read(lane_dir.join("report.pdf")).expect("existing output intact"),
            b"pre-existing"
        );
        assert_eq!(
            std::fs::read(lane_dir.join("guid-pub-report.pdf"))
                .expect("published under a non-clobbering name"),
            b"7-bytes"
        );
        assert!(!staged.exists(), "staging artifact is consumed");
        assert!(
            std::fs::read_dir(&lane_dir)
                .expect("lane dir readable")
                .flatten()
                .all(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    !name.ends_with(".crdownload")
                }),
            "no publication temp may survive in the lane output directory"
        );
    }

    /// P2 回归：发布失败(目标目录不可用)时回滚——不 finalize、staging 清理、
    /// route 的 reservation 被丢弃从而释放 active 记账。
    #[tokio::test]
    async fn finish_download_publish_failure_rolls_back_charge_and_releases_reservation() {
        let temp = tempfile::tempdir().expect("create publish failure root");
        let staging = temp.path().join("host-staging");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        // The lane output "directory" is actually a file: create_dir_all and
        // every publication step below it must fail.
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::write(&lane_dir, b"not-a-directory").expect("occupy lane dir path");
        let ledger = HostDownloadLedger::new(Some(staging.clone()));

        let reservation = RecordingTaskDownloadReservation::new();
        ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "guid-fail".into(),
                ledger_route(&lane_dir, "report.pdf", reservation.clone()),
            );
        let staged = staging.join("guid-fail");
        std::fs::write(&staged, b"payload").expect("stage artifact");

        assert!(!ledger.finish_download("guid-fail", &staged));
        assert!(!staged.exists(), "failed publication cleans its staging artifact");
        assert!(
            !reservation.is_finalized(),
            "a rolled-back publication must not commit the permanent charge"
        );
        assert_eq!(
            Arc::strong_count(&reservation),
            1,
            "the ledger drops its reservation so the active charge is released"
        );
        assert_eq!(ledger.pending_download_count(), 0);
    }

    /// P1(staging 隔离)回归：双 Host 各自独占 staging 目录,扫描器只扫本 Host
    /// 目录;active 与 retained-cancel GUID 的滞留工件绝不被按 mtime 清除。
    #[tokio::test]
    async fn staging_sweep_skips_owned_guids_and_never_crosses_hosts() {
        let temp = tempfile::tempdir().expect("create dual staging root");
        let staging_a = temp.path().join("host-a");
        let staging_b = temp.path().join("host-b");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging_a).expect("create staging a");
        std::fs::create_dir_all(&staging_b).expect("create staging b");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");
        assert_ne!(staging_a, staging_b, "hosts never share a staging directory");
        let ledger_a = HostDownloadLedger::new(Some(staging_a.clone()));

        // Active route + retained-cancel (rejected) artifacts in A.
        ledger_a
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "guid-active".into(),
                ledger_route(
                    &lane_dir,
                    "slow.bin",
                    Arc::new(TestTaskDownloadReservation),
                ),
            );
        assert!(ledger_a.quarantine_rejected_download("guid-retained"));
        std::fs::write(staging_a.join("guid-active"), b"active").expect("stage active");
        std::fs::write(staging_a.join("guid-active.crdownload"), b"active-part")
            .expect("stage active part");
        std::fs::write(staging_a.join("guid-retained"), b"retained").expect("stage retained");
        std::fs::write(staging_a.join("guid-unowned"), b"stale").expect("stage unowned");
        std::fs::write(staging_b.join("guid-other-host"), b"other").expect("stage other");

        let future = std::time::SystemTime::now()
            .checked_add(DOWNLOAD_ROUTE_TTL + Duration::from_secs(1))
            .expect("test clock remains representable");
        for _ in 0..4 {
            ledger_a.sweep_stale_staging_files_at(future);
            if !staging_a.join("guid-unowned").exists() {
                break;
            }
        }

        assert!(
            !staging_a.join("guid-unowned").exists(),
            "unowned stale artifacts are still reclaimed"
        );
        assert!(
            staging_a.join("guid-active").exists()
                && staging_a.join("guid-active.crdownload").exists(),
            "an active route's artifacts survive the age-based sweep"
        );
        assert!(
            staging_a.join("guid-retained").exists(),
            "a retained-cancel artifact survives until terminal proof"
        );
        assert!(
            staging_b.join("guid-other-host").exists(),
            "another Host's staging directory is never touched"
        );
    }

    /// P1(无 workspace 拒绝)回归:download_dir 为 None 的 Lane(即无受管
    /// workspace)在 admission 处 fail-closed;有 workspace 的对照 Lane 正常获准。
    #[tokio::test]
    async fn lane_without_workspace_denies_download_admission_fail_closed() {
        let temp = tempfile::tempdir().expect("create no-workspace test root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );

        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_target = Arc::new(AsyncMutex::new("tab-none".to_string()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane(
                "lane-none".into(),
                &tabs,
                &active_target,
                &active_frame,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await
            .expect("workspace-less lane registers");
        assert!(router.claim_target("lane-none", "tab-none").await);
        router.claim_frame("lane-none", "tab-none").await;
        assert!(
            !router.begin_download("tab-none", "guid-none", "file.pdf").await,
            "a lane without a managed workspace must not admit downloads"
        );

        let tabs_b = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_target_b = Arc::new(AsyncMutex::new("tab-some".to_string()));
        let active_frame_b = Arc::new(AsyncMutex::new(None));
        router
            .register_lane(
                "lane-some".into(),
                &tabs_b,
                &active_target_b,
                &active_frame_b,
                Arc::new(AtomicBool::new(false)),
                Some(lane_dir.to_string_lossy().into_owned()),
            )
            .await
            .expect("workspace lane registers");
        assert!(router.claim_target("lane-some", "tab-some").await);
        router.claim_frame("lane-some", "tab-some").await;
        assert!(
            router.begin_download("tab-some", "guid-some", "file.pdf").await,
            "the workspace-backed control lane admits the same download"
        );

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    /// P1(Host Drop 精确收敛)回归:hand_off 后,router/PendingDownload/reservation
    /// 的所有权跟随 DurableProcessCleanup 的 completion ticket;active reservation
    /// 只在 exact 进程树停止被证明后释放,且独占 staging 目录随之删除。
    #[cfg(any(windows, unix))]
    #[tokio::test]
    async fn handed_off_cleanup_retains_download_reservations_until_exact_stop_proof() {
        let (temp, profile, cleanup, pid) =
            durable_process_cleanup_fixture("hold-downloads-profile", None).await;
        // Staging lives outside the ephemeral profile so its removal below is
        // attributable to the post-stop reconcile, not to profile cleanup.
        let staging = temp.path().join("download-staging-host");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        let ledger = HostDownloadLedger::new(Some(staging.clone()));

        let reservation = RecordingTaskDownloadReservation::new();
        let observed = Arc::downgrade(&reservation);
        ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "guid-held".into(),
                ledger_route(&profile, "held.bin", reservation),
            );
        std::fs::write(staging.join("guid-held"), b"held").expect("stage held artifact");
        cleanup.install_post_stop_reconcile(
            Arc::clone(&ledger) as Arc<dyn crate::launch::HostStopReconcile>
        );

        assert!(
            observed.upgrade().is_some(),
            "the reservation is retained while no stop proof exists"
        );
        cleanup.hand_off();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), cleanup.finish())
                .await
                .expect("handed-off cleanup completes")
                .is_ok(),
            "the relay proves exact process-tree stop"
        );

        // Reconcile ran under stop proof: the pending route was drained, its
        // reservation dropped (releasing the active charge), staging removed.
        assert!(
            observed.upgrade().is_none(),
            "the retained reservation is released only after exact stop proof"
        );
        assert!(
            !staging.exists(),
            "the exclusive staging directory is removed after reconcile"
        );
        assert_eq!(ledger.pending_download_count(), 0);
        wait_for_durable_cleanup_process_exit(pid).await;
    }

    /// P1(取消不是终态)回归:Lane unregister 触发的 Browser.cancelDownload 被
    /// 拒绝(fake 返回协议错误)时——路由与 reservation 保持、Host 下载准入被
    /// 毒化、"Chromium 仍在写盘"的滞留工件不被按 mtime 清扫;只有 host-stop
    /// 终态化才释放 reservation 并回收工件。
    #[tokio::test]
    async fn cancel_failure_retains_reservation_until_host_stop_finalization() {
        let temp = tempfile::tempdir().expect("create cancel failure root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");

        // Minimal fake: Browser.cancelDownload → protocol error; anything
        // else → empty success.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cancel-failure fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            while let Some(Ok(message)) = futures_util::StreamExt::next(&mut websocket).await {
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                let mut response = if request["method"] == "Browser.cancelDownload" {
                    serde_json::json!({
                        "id": id,
                        "error": { "code": -32000, "message": "cancel refused by fixture" }
                    })
                } else {
                    serde_json::json!({ "id": id, "result": {} })
                };
                if let Some(session_id) = request.get("sessionId") {
                    response["sessionId"] = session_id.clone();
                }
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends response");
            }
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect cancel-failure fake websocket");
        let router = HostTargetRouter::new_with_download_staging(
            connection.clone(),
            staging.clone(),
        );

        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_target = Arc::new(AsyncMutex::new("tab-1".to_string()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        let registration = router
            .register_lane(
                "lane-a".into(),
                &tabs,
                &active_target,
                &active_frame,
                Arc::new(AtomicBool::new(false)),
                Some(lane_dir.to_string_lossy().into_owned()),
            )
            .await
            .expect("lane registers");
        let reservation = RecordingTaskDownloadReservation::new();
        router
            .download_ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "guid-writing".into(),
                ledger_route(&lane_dir, "writing.bin", reservation.clone()),
            );
        std::fs::write(staging.join("guid-writing"), b"still-being-written")
            .expect("stage in-flight artifact");

        assert!(
            router
                .unregister_lane_if_current("lane-a", registration)
                .await
        );
        // The cancel command was refused: the route stays retained-cancel,
        // its reservation is still held, and Host download admission is
        // poisoned fail-closed.
        assert_eq!(router.pending_download_count(), 1);
        assert!(router.download_cancel_requested("guid-writing"));
        assert!(
            Arc::strong_count(&reservation) > 1,
            "a refused cancel must not release the active reservation"
        );
        assert!(
            router
                .download_ledger
                .download_cleanup_poisoned
                .load(Ordering::Acquire),
            "a refused cancel poisons this Host's download admission"
        );

        // Chromium may keep writing: the age-based sweep must not reclaim
        // the owned artifact.
        let future = std::time::SystemTime::now()
            .checked_add(DOWNLOAD_ROUTE_TTL + Duration::from_secs(1))
            .expect("test clock remains representable");
        for _ in 0..4 {
            router.download_ledger.sweep_stale_staging_files_at(future);
        }
        assert!(
            staging.join("guid-writing").exists(),
            "a retained-cancel artifact survives the sweep while unproven"
        );

        // Only host-stop finalization (exact process-stop proof held by the
        // caller) releases the reservation and reclaims the artifact.
        assert_eq!(
            router.finalize_downloads_after_host_stop(),
            vec!["guid-writing"]
        );
        assert_eq!(Arc::strong_count(&reservation), 1);
        assert!(!reservation.is_finalized());
        assert!(!staging.join("guid-writing").exists());

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    /// P1(取消不是终态)回归:cancel 已发出(甚至已被 ack)但迟迟没有 terminal
    /// `downloadProgress` ——有界宽限期后 Host 下载面被毒化(触发上层
    /// poison+shutdown),期间 reservation/route 全程保留。
    #[tokio::test]
    async fn cancel_ack_without_terminal_event_poisons_after_bounded_grace() {
        let temp = tempfile::tempdir().expect("create cancel grace root");
        let staging = temp.path().join("host-staging");
        let lane_dir = temp.path().join("lane-downloads");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        std::fs::create_dir_all(&lane_dir).expect("create lane dir");
        let ledger = HostDownloadLedger::new(Some(staging.clone()));

        let reservation = RecordingTaskDownloadReservation::new();
        ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "guid-grace".into(),
                ledger_route(&lane_dir, "grace.bin", reservation.clone()),
            );
        // The cancel request was issued a full grace period ago and no
        // terminal event arrived.
        ledger
            .downloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut("guid-grace")
            .expect("route present")
            .cancel_requested_at =
            Some(std::time::Instant::now() - DOWNLOAD_CANCEL_TERMINAL_GRACE - Duration::from_secs(1));

        assert!(
            ledger.cancel_terminal_grace_expired(),
            "a cancel without terminal proof must poison after the bounded grace"
        );
        assert!(
            ledger.download_cleanup_poisoned.load(Ordering::Acquire),
            "grace expiry marks the Host download surface poisoned"
        );
        // The reservation and route stay retained: only host-stop
        // finalization releases them.
        assert_eq!(ledger.pending_download_count(), 1);
        assert!(Arc::strong_count(&reservation) > 1);
        assert_eq!(
            ledger.finalize_downloads_after_host_stop(),
            vec!["guid-grace"]
        );
        assert_eq!(Arc::strong_count(&reservation), 1);
        assert!(!reservation.is_finalized());
    }

    /// **F52**：`abort_tab_record` 契约——三个后台循环（inject/oopif/**debug**）
    /// 全部被 abort（tab 发现循环的重复插入丢弃路径此前漏掉 `_debug_loop`，
    /// 现已统一走本 helper）。
    #[tokio::test]
    async fn abort_tab_record_aborts_all_three_tab_loops() {
        let (connection, server) = router_test_connection().await;
        let mut record = test_tab_record(&connection, "tab-x", "session-x");
        let child_loop = tokio::spawn(std::future::pending::<()>());
        let child_abort = child_loop.abort_handle();
        record
            .oopif_managers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                "oopif-session".into(),
                OopifEntry {
                    target_id: "oopif-target".into(),
                    manager: InjectionManager::new(
                        connection.clone(),
                        "oopif-session",
                    ),
                    _loop: child_loop,
                },
            );

        abort_tab_record(&record);

        async fn assert_cancelled(
            name: &str,
            handle: &mut tokio::task::JoinHandle<()>,
        ) {
            let error = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .expect("aborted loop settles promptly")
                .expect_err("loop task must not complete normally");
            assert!(error.is_cancelled(), "{name} loop must be aborted, not leaked");
        }
        assert_cancelled("inject", &mut record._inject_loop).await;
        assert_cancelled("oopif", &mut record._oopif_loop).await;
        assert_cancelled("debug", &mut record._debug_loop).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !child_abort.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("tab teardown aborts every retained OOPIF child loop");
        assert!(
            record
                .oopif_managers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "tab teardown drains its OOPIF map"
        );

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn oopif_detach_and_destroy_remove_and_abort_exact_entries() {
        let (connection, server) = router_test_connection().await;
        let managers = Arc::new(Mutex::new(HashMap::new()));
        let mut aborts = Vec::new();
        for (session, target) in [
            ("session-a", "target-a"),
            ("session-b", "target-b"),
            ("session-c", "target-b"),
        ] {
            let loop_handle = tokio::spawn(std::future::pending::<()>());
            aborts.push(loop_handle.abort_handle());
            managers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    session.into(),
                    OopifEntry {
                        target_id: target.into(),
                        manager: InjectionManager::new(connection.clone(), session),
                        _loop: loop_handle,
                    },
                );
        }

        assert!(remove_oopif_session(&managers, "session-a"));
        assert_eq!(remove_oopif_target(&managers, "target-b"), 2);
        assert!(
            managers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while aborts.iter().any(|abort| !abort.is_finished()) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("removed OOPIF entries abort their injection loops");

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    /// F53 专用 fake：`createTarget` → 先回 nonce 关联的 attach 事件再回 targetId；
    /// `Page.getFrameTree` → 单帧树；`Page.navigate` → CDP error（模拟导航失败）；
    /// `closeTarget` → success:true；`getTargets` → 空；其它 → {}（回显 sessionId）。
    async fn navigate_failure_open_link_fake_connection() -> (
        Connection,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind navigate-failure fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            while let Some(Ok(message)) = futures_util::StreamExt::next(&mut websocket).await {
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                let method = request["method"].as_str().unwrap_or_default();
                let response = match method {
                    "Target.createTarget" => {
                        let url = request["params"]["url"].as_str().unwrap_or_default();
                        // flatten auto-attach 模拟：先发 nonce 关联 attach 事件。
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({
                                    "method": "Target.attachedToTarget",
                                    "params": {
                                        "sessionId": "pending-session",
                                        "targetInfo": {
                                            "targetId": "pending-target",
                                            "type": "page",
                                            "title": "",
                                            "url": url,
                                            "attached": true,
                                            "canAccessOpener": false
                                        },
                                        "waitingForDebugger": false
                                    }
                                })
                                .to_string()
                                .into(),
                            ),
                        )
                        .await
                        .expect("fake sends nonce-correlated attach");
                        serde_json::json!({ "id": id, "result": { "targetId": "pending-target" } })
                    }
                    "Page.getFrameTree" => serde_json::json!({
                        "id": id,
                        "result": { "frameTree": { "frame": { "id": "pending-target" } } }
                    }),
                    "Page.navigate" => serde_json::json!({
                        "id": id,
                        "error": { "code": -32000, "message": "net::ERR_FAILED (fake)" }
                    }),
                    "Target.closeTarget" => serde_json::json!({
                        "id": id,
                        "result": { "success": true }
                    }),
                    "Target.getTargets" => serde_json::json!({
                        "id": id,
                        "result": { "targetInfos": [] }
                    }),
                    _ => serde_json::json!({ "id": id, "result": {} }),
                };
                let mut response = response;
                if let Some(session_id) = request.get("sessionId") {
                    response["sessionId"] = session_id.clone();
                }
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends response");
            }
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect navigate-failure fake websocket");
        (connection, server)
    }

    /// **F53 回归**：standalone 后端（无 router loss 事件路径）的
    /// `open_link_new_tab` 在 navigate 失败时不得留下幽灵 TabRecord——
    /// PendingCreatedPage 清理会关掉 target，tabs 里的残留记录会让 tabs 永远列出
    /// 一个死 tab、switch 过去后一切操作 TargetClosed。
    #[tokio::test]
    async fn standalone_open_link_navigate_failure_leaves_no_ghost_tab() {
        let (connection, server) = navigate_failure_open_link_fake_connection().await;
        let backend = test_backend_with_tabs(connection.clone(), &["tab-main"]);

        backend
            .act_open_link_new_tab("https://example.com/next")
            .await
            .expect_err("navigate failure must surface to the caller");

        assert!(
            !backend.tabs.lock().await.contains_key("pending-target"),
            "the armed record must be scrubbed on navigate failure (no ghost tab)"
        );

        drop(backend);
        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    async fn withheld_create_response_fake_connection(
        send_attach: bool,
        delayed_inventory_visibility: bool,
    ) -> (
        Connection,
        tokio::sync::mpsc::UnboundedReceiver<String>,
        Arc<AtomicBool>,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind pending-create fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (method_tx, method_rx) = tokio::sync::mpsc::unbounded_channel();
        let target_live = Arc::new(AtomicBool::new(false));
        let server_target_live = Arc::clone(&target_live);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut methods = Vec::new();
            let mut pending_url = None::<String>;
            let mut inventory_calls = 0usize;
            while let Some(message) = futures_util::StreamExt::next(&mut websocket).await {
                let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                let method = request["method"]
                    .as_str()
                    .expect("fake request has method")
                    .to_string();
                methods.push(method.clone());
                let _ = method_tx.send(method.clone());
                match method.as_str() {
                    "Target.createTarget" => {
                        let url = request["params"]["url"]
                            .as_str()
                            .expect("createTarget carries nonce url")
                            .to_string();
                        pending_url = Some(url.clone());
                        server_target_live.store(true, Ordering::Release);
                        if send_attach {
                            futures_util::SinkExt::send(
                                &mut websocket,
                                tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::json!({
                                        "method": "Target.attachedToTarget",
                                        "params": {
                                            "sessionId": "pending-session",
                                            "targetInfo": {
                                                "targetId": "pending-target",
                                                "type": "page",
                                                "title": "",
                                                "url": url,
                                                "attached": true,
                                                "canAccessOpener": false
                                            },
                                            "waitingForDebugger": false
                                        }
                                    })
                                    .to_string()
                                    .into(),
                                ),
                            )
                            .await
                            .expect("fake sends nonce-correlated attach");
                        }
                        if delayed_inventory_visibility {
                            futures_util::SinkExt::send(
                                &mut websocket,
                                tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::json!({
                                        "id": id,
                                        "error": {
                                            "code": -32001,
                                            "message": "create response lost after execution"
                                        }
                                    })
                                    .to_string()
                                    .into(),
                                ),
                            )
                            .await
                            .expect("fake terminates unknown create command");
                        }
                        // Otherwise deliberately withhold the response: the
                        // nonce-correlated attach remains exact authority.
                    }
                    "Target.closeTarget" => {
                        assert_eq!(request["params"]["targetId"], "pending-target");
                        server_target_live.store(false, Ordering::Release);
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({
                                    "id": id,
                                    "result": { "success": true }
                                })
                                .to_string()
                                .into(),
                            ),
                        )
                        .await
                        .expect("fake confirms pending target close");
                    }
                    "Browser.close" => {
                        server_target_live.store(false, Ordering::Release);
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({
                                    "id": id,
                                    "result": {}
                                })
                                .to_string()
                                .into(),
                            ),
                        )
                        .await
                        .expect("fake confirms whole-browser close");
                    }
                    "Target.getTargets" => {
                        inventory_calls += 1;
                        if delayed_inventory_visibility && inventory_calls == 1 {
                            futures_util::SinkExt::send(
                                &mut websocket,
                                tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::json!({
                                        "id": id,
                                        "error": {
                                            "code": -32000,
                                            "message": "transient inventory failure"
                                        }
                                    })
                                    .to_string()
                                    .into(),
                                ),
                            )
                            .await
                            .expect("fake sends transient inventory failure");
                            continue;
                        }
                        let target_infos = if server_target_live.load(Ordering::Acquire) {
                            if delayed_inventory_visibility && inventory_calls == 2 {
                                Vec::new()
                            } else {
                                vec![serde_json::json!({
                                    "targetId": "pending-target",
                                    "type": "page",
                                    "title": "",
                                    "url": pending_url.as_deref().expect("create url recorded")
                                })]
                            }
                        } else {
                            Vec::new()
                        };
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({
                                    "id": id,
                                    "result": { "targetInfos": target_infos }
                                })
                                .to_string()
                                .into(),
                            ),
                        )
                        .await
                        .expect("fake returns target inventory");
                    }
                    other => panic!("unexpected pending-create fake CDP method {other}"),
                }
            }
            methods
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect pending-create fake websocket");
        (connection, method_rx, target_live, server)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_withheld_create_response_closes_nonce_correlated_target_and_scrubs_router() {
        let (connection, mut methods, target_live, server) =
            withheld_create_response_fake_connection(true, false).await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();
        tokio::time::timeout(Duration::from_secs(2), async {
            while router.cleanup_executor.worker_count() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("one bounded Host cleanup worker starts");
        let launch = {
            let connection = connection.clone();
            let router = Arc::clone(&router);
            let executor = Arc::clone(&router.cleanup_executor);
            tokio::spawn(async move {
                create_pending_page_session_owned(connection, executor, Some(router), true, None)
                    .await
            })
        };

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("createTarget reaches fake server"),
            Some("Target.createTarget".into())
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if router
                    .state
                    .lock()
                    .await
                    .quarantined
                    .contains_key("pending-target")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("nonce target is quarantined before caller cancellation");

        launch.abort();
        let _ = launch.await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), methods.recv())
                .await
                .expect("independent cleanup closes target"),
            Some("Target.closeTarget".into())
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = router.state.lock().await;
                let scrubbed = !state.quarantined.contains_key("pending-target")
                    && !state
                        .session_targets
                        .values()
                        .any(|target| target == "pending-target");
                drop(state);
                if scrubbed && !target_live.load(Ordering::Acquire) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closed nonce target and router quarantine are absent");

        connection.shutdown().await;
        router_loop.abort();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_create_response_escalates_host_without_accepting_empty_inventory() {
        let (connection, mut methods, target_live, server) =
            withheld_create_response_fake_connection(false, true).await;
        let router = HostTargetRouter::new(connection.clone());
        let launch = {
            let connection = connection.clone();
            let router = Arc::clone(&router);
            let executor = Arc::clone(&router.cleanup_executor);
            tokio::spawn(async move {
                create_pending_page_session_owned(connection, executor, Some(router), true, None)
                    .await
            })
        };
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("createTarget reaches fake server"),
            Some("Target.createTarget".into())
        );
        assert!(
            launch
                .await
                .expect("owned create task joins")
                .is_err(),
            "the fake create response is terminally lost"
        );

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("unknown identity triggers authoritative shutdown"),
            Some("Browser.close".into()),
            "a transient empty inventory must never complete an unknown create cleanup"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while target_live.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("whole-Host shutdown subsumes the delayed nonce target");

        connection.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unpublished_backend_drop_is_generation_safe_and_duplicate_registration_fails_closed()
    {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(true)],
            vec![Vec::new(), Vec::new()],
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();
        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_target = Arc::new(AsyncMutex::new("old-target".to_string()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        let lane_closing = Arc::new(AtomicBool::new(false));
        let old_registration = router
            .register_lane(
                "same-lane".into(),
                &tabs,
                &active_target,
                &active_frame,
                Arc::clone(&lane_closing),
                None,
            )
            .await
            .expect("first generation registers");

        let replacement_tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let replacement_active = Arc::new(AsyncMutex::new("new-target".to_string()));
        let replacement_frame = Arc::new(AsyncMutex::new(None));
        assert!(
            router
                .register_lane(
                    "same-lane".into(),
                    &replacement_tabs,
                    &replacement_active,
                    &replacement_frame,
                    Arc::new(AtomicBool::new(false)),
                    None,
                )
                .await
                .is_none(),
            "a same-id reopen must never overwrite a cleaning generation"
        );
        assert!(
            router
                .is_current_registration("same-lane", old_registration)
                .await
        );
        assert!(router.claim_target("same-lane", "old-target").await);
        router.claim_frame("same-lane", "old-frame").await;

        let cleanup = LaneCleanupAuthority::new(
            connection.clone(),
            Arc::clone(&router.cleanup_executor),
            Arc::clone(&router),
            "same-lane".into(),
            Arc::clone(&lane_closing),
            Arc::clone(&tabs),
            "old-target".into(),
            "old-session".into(),
            "old-frame".into(),
            None,
            None,
        );
        cleanup.set_registration(old_registration);
        let mut unpublished_backend = test_backend_with_tabs(connection.clone(), &[]);
        unpublished_backend.test_router = Some(Arc::clone(&router));
        unpublished_backend.lane_id = "same-lane".into();
        unpublished_backend.lane_cleanup = Some(Arc::clone(&cleanup));
        unpublished_backend.lane_closing = Arc::clone(&lane_closing);
        unpublished_backend.lane_closed = Arc::new(AtomicBool::new(false));
        unpublished_backend.tabs = Arc::clone(&tabs);
        unpublished_backend.active_target = Arc::clone(&active_target);
        unpublished_backend.active_frame = Arc::clone(&active_frame);
        PendingLaneLaunchGuard::new(Arc::clone(&cleanup)).commit();
        // Models cancellation in HostLaneCoordinator::insert_if_open after
        // from_host returned but before the backend was published.
        drop(unpublished_backend);
        tokio::time::timeout(Duration::from_secs(5), async {
            while cleanup.state.load(Ordering::Acquire) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("old generation cleanup completes");

        {
            let state = router.state.lock().await;
            assert!(!state.lanes.contains_key("same-lane"));
            assert_eq!(state.ownership.owner("old-target"), None);
            assert!(!state.quarantined.contains_key("old-target"));
            assert!(!state
                .session_targets
                .values()
                .any(|target| target == "old-target"));
            assert_eq!(state.frame_owner.get("old-frame"), None);
        }

        let new_registration = router
            .register_lane(
                "same-lane".into(),
                &replacement_tabs,
                &replacement_active,
                &replacement_frame,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await
            .expect("same id may register only after exact old cleanup");
        cleanup.hand_off();
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(
            router.cleanup_executor.worker_count(),
            1,
            "repeated cleanup handoff must not create another OS worker"
        );
        assert!(
            router
                .is_current_registration("same-lane", new_registration)
                .await,
            "completed old cleanup must not remove the new generation"
        );
        assert!(
            !router
                .unregister_lane_if_current("same-lane", old_registration)
                .await,
            "an old registration token cannot unregister the new generation"
        );
        assert!(
            router
                .unregister_lane_if_current("same-lane", new_registration)
                .await
        );

        connection.shutdown().await;
        router_loop.abort();
        server.abort();
        let _ = server.await;
    }

    async fn saturated_cleanup_fake_connection() -> (
        Connection,
        tokio::sync::mpsc::UnboundedReceiver<String>,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind saturated-cleanup fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (method_tx, method_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut methods = Vec::new();
            while let Some(message) = futures_util::StreamExt::next(&mut websocket).await {
                let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                let method = request["method"]
                    .as_str()
                    .expect("fake request has method")
                    .to_string();
                methods.push(method.clone());
                let _ = method_tx.send(method.clone());
                match method.as_str() {
                    // Permanently withhold the first exact-target close. The
                    // executor must not spawn another cleanup future around it.
                    "Target.closeTarget" => {}
                    "Browser.close" => {
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({ "id": id, "result": {} })
                                    .to_string()
                                    .into(),
                            ),
                        )
                        .await
                        .expect("fake acknowledges authoritative browser close");
                    }
                    other => panic!("unexpected saturated-cleanup CDP method {other}"),
                }
            }
            methods
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect saturated-cleanup fake websocket");
        (connection, method_rx, server)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn single_permanent_cleanup_failure_exhausts_budget_and_closes_host() {
        let (connection, mut methods, server) = saturated_cleanup_fake_connection().await;
        let executor = TargetCleanupExecutor::new(connection.clone(), None)
            .expect("bounded cleanup executor starts");
        let scope = crate::host::StandaloneResourceScope::new();
        let lane_authority = scope
            .reserve_lane("pending-page-cleanup".into())
            .expect("pending Lane structural slot");
        let sibling_lanes = (1..crate::host::STANDALONE_MAX_LIVE_LANES_PER_SCOPE)
            .map(|index| {
                scope
                    .reserve_lane(format!("sibling-{index}"))
                    .expect("sibling Lane within cap")
            })
            .collect::<Vec<_>>();
        let cleanup = PendingCreatedPageCleanup::new(
            connection.clone(),
            Arc::clone(&executor),
            None,
            Some("only-stuck-target".into()),
            Some("only-stuck-session".into()),
            None,
            None,
            Some(lane_authority),
        );
        cleanup.hand_off();
        assert!(
            scope.reserve_lane("n-plus-one-before-cleanup".into()).is_err(),
            "caller cancellation must not return the Lane slot while its target cleanup is pending"
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("single cleanup command reaches fake server"),
            Some("Target.closeTarget".into())
        );
        assert_eq!(
            tokio::time::timeout(
                TARGET_CLEANUP_JOB_BUDGET + Duration::from_secs(2),
                methods.recv()
            )
            .await
            .expect("single stuck cleanup escalates after its budget"),
            Some("Browser.close".into())
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while cleanup.state.load(Ordering::Acquire) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Host-close proof completes the single exact intent");
        let replacement = scope
            .reserve_lane("replacement-after-host-proof".into())
            .expect("authoritative Host-close proof returns the pending Lane slot");
        assert!(executor.is_poisoned());
        assert_eq!(executor.max_active_jobs(), 1);
        assert!(executor.max_queued_jobs() <= 1);
        assert!(executor.ensure_accepting().is_err());

        drop(replacement);
        drop(sibling_lanes);
        assert_eq!(scope.counts(), (0, 0, 0));
        drop(executor);
        connection.shutdown().await;
        let methods = server.await.expect("single stuck-cleanup fake joins");
        assert_eq!(methods, vec!["Target.closeTarget", "Browser.close"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cleanup_only_router_target_uses_same_budget_and_never_spawns_an_unbounded_task() {
        let (connection, mut methods, server) = saturated_cleanup_fake_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        router
            .cleanup_executor
            .submit(TargetCleanupJob::RouterTarget {
                router: Arc::clone(&router),
                lane_id: "retired-lane".into(),
                target_id: "late-popup".into(),
            });
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("cleanup-only close reaches fake server"),
            Some("Target.closeTarget".into())
        );
        assert_eq!(
            tokio::time::timeout(
                TARGET_CLEANUP_JOB_BUDGET + Duration::from_secs(2),
                methods.recv()
            )
            .await
            .expect("cleanup-only target exhausts the common budget"),
            Some("Browser.close".into())
        );
        assert!(router.cleanup_executor.is_poisoned());
        assert_eq!(router.cleanup_executor.max_active_jobs(), 1);
        assert!(router.cleanup_executor.max_queued_jobs() <= 1);

        connection.shutdown().await;
        drop(router);
        let methods = server.await.expect("cleanup-only fake joins");
        assert_eq!(methods, vec!["Target.closeTarget", "Browser.close"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn permanent_cleanup_failure_has_bounded_queue_and_escalates_overflow_to_host_close() {
        let (connection, mut methods, server) = saturated_cleanup_fake_connection().await;
        let executor = TargetCleanupExecutor::new(connection.clone(), None)
            .expect("bounded cleanup executor starts");
        let mut cleanups = Vec::with_capacity(TARGET_CLEANUP_QUEUE_CAPACITY + 2);

        let active = PendingCreatedPageCleanup::new(
            connection.clone(),
            Arc::clone(&executor),
            None,
            Some("stuck-target".into()),
            Some("stuck-session".into()),
            None,
            None,
            None,
        );
        active.hand_off();
        cleanups.push(active);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("first cleanup command reaches fake server"),
            Some("Target.closeTarget".into())
        );

        // The active close never resolves. Exactly CAPACITY jobs may wait;
        // one additional cleanup is the deterministic overflow authority.
        for index in 0..=TARGET_CLEANUP_QUEUE_CAPACITY {
            let cleanup = PendingCreatedPageCleanup::new(
                connection.clone(),
                Arc::clone(&executor),
                None,
                Some(format!("queued-target-{index}")),
                Some(format!("queued-session-{index}")),
                None,
                None,
                None,
            );
            cleanup.hand_off();
            cleanups.push(cleanup);
        }

        tokio::time::timeout(Duration::from_secs(3), async {
            while !executor.is_poisoned()
                || cleanups
                    .iter()
                    .any(|cleanup| cleanup.state.load(Ordering::Acquire) != 2)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("overflow is retained until Host-close authority completes");

        assert_eq!(
            executor.max_active_jobs(),
            1,
            "permanent failure may occupy only the sole cleanup future"
        );
        assert_eq!(
            executor.max_queued_jobs(),
            TARGET_CLEANUP_QUEUE_CAPACITY,
            "the cleanup mailbox must never grow beyond its hard capacity"
        );
        assert!(
            executor.ensure_accepting().is_err(),
            "a poisoned Host must reject every later target/Lane create"
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("overflow reaches browser-wide shutdown"),
            Some("Browser.close".into())
        );
        assert!(
            matches!(
                tokio::time::timeout(Duration::from_millis(100), methods.recv()).await,
                Err(_) | Ok(None)
            ),
            "no second target cleanup command may run concurrently"
        );

        drop(executor);
        connection.shutdown().await;
        let methods = server.await.expect("saturated-cleanup fake joins");
        assert_eq!(
            methods,
            vec!["Target.closeTarget", "Browser.close"],
            "overflow must replace unbounded retry/concurrency with one Host close"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lane_cleanup_handoff_queue_overflow_escalates_to_host_close() {
        let (connection, mut methods, server) = saturated_cleanup_fake_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let executor = Arc::clone(&router.cleanup_executor);

        let active = PendingCreatedPageCleanup::new(
            connection.clone(),
            Arc::clone(&executor),
            None,
            Some("stuck-before-lane-handoff".into()),
            Some("stuck-before-lane-session".into()),
            None,
            None,
            None,
        );
        active.hand_off();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("active exact-target cleanup reaches fake server"),
            Some("Target.closeTarget".into())
        );

        let mut queued = Vec::with_capacity(TARGET_CLEANUP_QUEUE_CAPACITY);
        for index in 0..TARGET_CLEANUP_QUEUE_CAPACITY {
            let cleanup = PendingCreatedPageCleanup::new(
                connection.clone(),
                Arc::clone(&executor),
                None,
                Some(format!("queued-before-lane-{index}")),
                Some(format!("queued-before-lane-session-{index}")),
                None,
                None,
                None,
            );
            cleanup.hand_off();
            queued.push(cleanup);
        }

        let lane_cleanup = LaneCleanupAuthority::new(
            connection.clone(),
            Arc::clone(&executor),
            Arc::clone(&router),
            "failed-coordinator-lane".into(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AsyncMutex::new(HashMap::new())),
            "failed-lane-target".into(),
            "failed-lane-session".into(),
            "failed-lane-frame".into(),
            None,
            None,
        );
        lane_cleanup.hand_off();

        assert!(executor.is_poisoned());
        assert_eq!(
            executor.max_queued_jobs(),
            TARGET_CLEANUP_QUEUE_CAPACITY
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("Lane handoff overflow reaches browser-wide shutdown"),
            Some("Browser.close".into())
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while lane_cleanup.state.load(Ordering::Acquire) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Host-close proof finishes the overflowed Lane authority");

        drop(queued);
        drop(active);
        connection.shutdown().await;
        drop(router);
        let methods = server.await.expect("Lane-overflow fake joins");
        assert_eq!(methods, vec!["Target.closeTarget", "Browser.close"]);
    }

    async fn close_target_fake_connection(
        outcomes: Vec<Result<(), TransportError>>,
    ) -> (Connection, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind close-target fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut outcomes = outcomes.into_iter();
            let mut closed_targets = Vec::new();
            while let Some(message) = futures_util::StreamExt::next(&mut websocket).await {
                let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .expect("fake request has id");
                if request.get("method").and_then(serde_json::Value::as_str)
                    == Some("Target.closeTarget")
                {
                    if let Some(target_id) = request
                        .get("params")
                        .and_then(|params| params.get("targetId"))
                        .and_then(serde_json::Value::as_str)
                    {
                        closed_targets.push(target_id.to_string());
                    }
                    match outcomes.next().unwrap_or(Ok(())) {
                        Ok(()) => {
                            futures_util::SinkExt::send(
                                &mut websocket,
                                tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::json!({
                                        "id": id,
                                        "result": { "success": true }
                                    })
                                    .to_string()
                                    .into(),
                                ),
                            )
                            .await
                            .expect("fake sends closeTarget success");
                        }
                        Err(TransportError::Cdp { code, message }) => {
                            futures_util::SinkExt::send(
                                &mut websocket,
                                tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::json!({
                                        "id": id,
                                        "error": { "code": code, "message": message }
                                    })
                                    .to_string()
                                    .into(),
                                ),
                            )
                            .await
                            .expect("fake sends closeTarget cdp error");
                        }
                        Err(other) => panic!("fake closeTarget supports only Cdp errors, got {other:?}"),
                    }
                } else {
                    futures_util::SinkExt::send(
                        &mut websocket,
                        tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::json!({ "id": id, "result": {} })
                                .to_string()
                                .into(),
                        ),
                    )
                    .await
                    .expect("fake sends generic success");
                }
            }
            closed_targets
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect close-target fake websocket");
        (connection, server)
    }

    enum FakeCloseReply {
        Success(bool),
        Raw(serde_json::Value),
        Error(TransportError),
    }

    async fn close_target_inventory_fake_connection(
        close_result: FakeCloseReply,
        visible_targets: Vec<&str>,
    ) -> (Connection, tokio::task::JoinHandle<Vec<String>>) {
        close_target_inventory_fake_connection_with_infos(
            close_result,
            visible_targets
                .into_iter()
                .map(|target_id| serde_json::json!({ "targetId": target_id }))
                .collect(),
        )
        .await
    }

    async fn close_target_inventory_fake_connection_with_infos(
        close_result: FakeCloseReply,
        target_infos: Vec<serde_json::Value>,
    ) -> (Connection, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind close-target inventory fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut methods = Vec::new();
            while let Some(message) = futures_util::StreamExt::next(&mut websocket).await {
                let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .expect("fake request has id");
                let method = request
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .expect("fake request has method")
                    .to_string();
                methods.push(method.clone());
                let response = match method.as_str() {
                    "Target.closeTarget" => match &close_result {
                        FakeCloseReply::Success(success) => serde_json::json!({
                            "id": id,
                            "result": { "success": success }
                        }),
                        FakeCloseReply::Raw(result) => serde_json::json!({
                            "id": id,
                            "result": result
                        }),
                        FakeCloseReply::Error(TransportError::Cdp { code, message }) => serde_json::json!({
                            "id": id,
                            "error": { "code": code, "message": message }
                        }),
                        FakeCloseReply::Error(other) => {
                            panic!("inventory fake supports only Cdp errors, got {other:?}")
                        }
                    },
                    "Target.getTargets" => serde_json::json!({
                        "id": id,
                        "result": {
                            "targetInfos": target_infos.clone()
                        }
                    }),
                    other => panic!("unexpected fake CDP method {other}"),
                };
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends response");
            }
            methods
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect close-target inventory fake websocket");
        (connection, server)
    }

    async fn scripted_close_target_fake_connection(
        close_replies: Vec<FakeCloseReply>,
        inventories: Vec<Vec<serde_json::Value>>,
    ) -> (Connection, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind scripted close-target fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut close_replies = std::collections::VecDeque::from(close_replies);
            let mut inventories = std::collections::VecDeque::from(inventories);
            let mut methods = Vec::new();
            while let Some(message) = futures_util::StreamExt::next(&mut websocket).await {
                let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                let method = request["method"]
                    .as_str()
                    .expect("fake request has method")
                    .to_string();
                methods.push(method.clone());
                let response = match method.as_str() {
                    "Target.closeTarget" => match close_replies
                        .pop_front()
                        .expect("script provides every close reply")
                    {
                        FakeCloseReply::Success(success) => serde_json::json!({
                            "id": id,
                            "result": { "success": success }
                        }),
                        FakeCloseReply::Raw(result) => {
                            serde_json::json!({ "id": id, "result": result })
                        }
                        FakeCloseReply::Error(TransportError::Cdp { code, message }) => {
                            serde_json::json!({
                                "id": id,
                                "error": { "code": code, "message": message }
                            })
                        }
                        FakeCloseReply::Error(other) => {
                            panic!("scripted fake supports only Cdp errors, got {other:?}")
                        }
                    },
                    "Target.getTargets" => serde_json::json!({
                        "id": id,
                        "result": {
                            "targetInfos": inventories
                                .pop_front()
                                .expect("script provides every inventory")
                        }
                    }),
                    other => panic!("unexpected scripted fake CDP method {other}"),
                };
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends response");
            }
            methods
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect scripted close-target fake websocket");
        (connection, server)
    }

    async fn detached_target_cleanup_fake_connection() -> (
        Connection,
        tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
        tokio::sync::mpsc::UnboundedReceiver<String>,
        Arc<tokio::sync::Notify>,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind detached-target fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (event_tx, mut event_rx) =
            tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
        let (method_tx, method_rx) = tokio::sync::mpsc::unbounded_channel();
        let allow_exact_close = Arc::new(tokio::sync::Notify::new());
        let allow_exact_close_for_server = Arc::clone(&allow_exact_close);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut methods = Vec::new();
            let mut close_attempt = 0usize;
            let mut events_open = true;
            loop {
                tokio::select! {
                    biased;
                    event = event_rx.recv(), if events_open => {
                        let Some(event) = event else {
                            events_open = false;
                            continue;
                        };
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                event.to_string().into(),
                            ),
                        )
                        .await
                        .expect("fake sends target event");
                    }
                    message = futures_util::StreamExt::next(&mut websocket) => {
                        let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = message else {
                            break;
                        };
                        let request: serde_json::Value =
                            serde_json::from_str(&text).expect("fake received valid json");
                        let id = request["id"].as_u64().expect("fake request has id");
                        let method = request["method"]
                            .as_str()
                            .expect("fake request has method")
                            .to_string();
                        methods.push(method.clone());
                        let _ = method_tx.send(method.clone());
                        let response = match method.as_str() {
                            "Target.closeTarget" => {
                                close_attempt += 1;
                                if close_attempt == 1 {
                                    serde_json::json!({ "id": id, "result": { "success": false } })
                                } else {
                                    allow_exact_close_for_server.notified().await;
                                    serde_json::json!({ "id": id, "result": { "success": true } })
                                }
                            }
                            "Target.getTargets" => serde_json::json!({
                                "id": id,
                                "result": {
                                    "targetInfos": [{
                                        "targetId": "detached-target",
                                        "type": "page",
                                        "title": "still live",
                                        "url": "about:blank",
                                        "attached": false,
                                        "canAccessOpener": false
                                    }]
                                }
                            }),
                            other => panic!("unexpected detached-target fake CDP method {other}"),
                        };
                        futures_util::SinkExt::send(
                            &mut websocket,
                            tokio_tungstenite::tungstenite::Message::Text(
                                response.to_string().into(),
                            ),
                        )
                        .await
                        .expect("fake sends response");
                    }
                }
            }
            methods
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect detached-target fake websocket");
        (
            connection,
            event_tx,
            method_rx,
            allow_exact_close,
            server,
        )
    }

    async fn foreground_fake_connection() -> (
        Connection,
        tokio::task::JoinHandle<Vec<serde_json::Value>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind foreground fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut requests = Vec::new();
            while requests.len() < 4 {
                let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    futures_util::StreamExt::next(&mut websocket).await
                else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .expect("fake request has id");
                let method = request
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .expect("fake request has method");
                let result = if method == "Browser.getWindowForTarget" {
                    serde_json::json!({
                        "windowId": 73,
                        "bounds": {
                            "left": 80,
                            "top": 80,
                            "width": 1280,
                            "height": 800,
                            "windowState": "minimized"
                        }
                    })
                } else {
                    serde_json::json!({})
                };
                let mut response = serde_json::json!({ "id": id, "result": result });
                if let Some(session_id) = request.get("sessionId") {
                    response["sessionId"] = session_id.clone();
                }
                requests.push(request);
                futures_util::SinkExt::send(
                    &mut websocket,
                    tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ),
                )
                .await
                .expect("fake sends foreground response");
            }
            requests
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect foreground fake websocket");
        (connection, server)
    }

    fn inert_loop() -> tokio::task::JoinHandle<()> {
        tokio::spawn(std::future::pending::<()>())
    }

    fn test_tab_record(conn: &Connection, target_id: &str, session_id: &str) -> TabRecord {
        TabRecord {
            _task_tab_reservation: None,
            target_id: target_id.to_string(),
            session_id: session_id.to_string(),
            injection: InjectionManager::new(conn.clone(), session_id.to_string()),
            _inject_loop: inert_loop(),
            main_frame_id: target_id.to_string(),
            oopif_managers: Arc::new(Mutex::new(HashMap::new())),
            _oopif_loop: inert_loop(),
            ref_table: Arc::new(AsyncMutex::new(None)),
            debug: Arc::new(std::sync::Mutex::new(
                crate::debug_capture::DebugBuffers::default(),
            )),
            _debug_loop: inert_loop(),
        }
    }

    fn test_backend_with_tabs(conn: Connection, target_ids: &[&str]) -> CdpBackend {
        let tabs = target_ids
            .iter()
            .map(|target_id| {
                (
                    (*target_id).to_string(),
                    test_tab_record(&conn, target_id, &format!("session-{target_id}")),
                )
            })
            .collect::<HashMap<_, _>>();
        let cleanup_executor = TargetCleanupExecutor::new(conn.clone(), None)
            .expect("test target cleanup executor starts");
        CdpBackend {
            conn,
            host: None,
            test_router: None,
            lane_id: "test-lane".to_string(),
            task_tab_reservation_scope: None,
            task_download_reservation_scope: None,
            reliable_event_task_budget: ReliableEventTaskBudget::new_opaque(),
            lane_cleanup: None,
            cleanup_executor,
            lane_closing: Arc::new(AtomicBool::new(false)),
            lane_closed: Arc::new(AtomicBool::new(false)),
            lane_retired: AtomicBool::new(false),
            lane_shutdown_gate: AsyncMutex::new(()),
            lane_close_confirmed: AsyncMutex::new(HashSet::new()),
            lane_cancel: CancellationToken::new(),
            tabs: Arc::new(AsyncMutex::new(tabs)),
            active_target: Arc::new(AsyncMutex::new(
                target_ids.first().copied().unwrap_or_default().to_string(),
            )),
            active_frame: Arc::new(AsyncMutex::new(None)),
            act_seq: AtomicU64::new(0),
            _process: None,
            _attach_loop: None,
            _tab_discovery_loop: None,
            _download_loop: None,
            download_dir: None,
            workspace_dir: None,
            _firewall_runtime: None,
            firewall_config: crate::firewall::FirewallConfig::default(),
            approved_domains: crate::firewall::ApprovedDomains::new(),
            evaluate_gate: AsyncMutex::new(crate::evaluate::EvaluateGate::default()),
            headful: false,
            display_available: false,
            op_mutex: LaneOperationGate::default(),
            target_recovery_gate: AsyncMutex::new(()),
            known_secret_values: crate::KnownSecretValues::default(),
        }
    }

    #[tokio::test]
    async fn dropping_standalone_backend_aborts_owned_loops_and_closes_transport() {
        let (connection, requests, server) = generic_recording_fake_connection().await;
        let mut backend = test_backend_with_tabs(connection, &[]);
        let attach_loop = backend.conn.run_attach_loop();
        let attach_abort = attach_loop.abort_handle();
        let discovery_loop = tokio::spawn(std::future::pending::<()>());
        let discovery_abort = discovery_loop.abort_handle();
        let download_loop = spawn_download_loop(backend.conn.clone(), None);
        let download_abort = download_loop.abort_handle();
        backend._attach_loop = Some(attach_loop);
        backend._tab_discovery_loop = Some(discovery_loop);
        backend._download_loop = Some(download_loop);

        drop(backend);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !attach_abort.is_finished()
                || !discovery_abort.is_finished()
                || !download_abort.is_finished()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("standalone Drop explicitly aborts every owned loop");

        drop(requests);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("all retained Connection clones converge after backend Drop")
            .expect("fake transport task joins");
    }

    #[tokio::test]
    async fn bring_to_front_restores_real_window_before_activating_target() {
        let (connection, server) = foreground_fake_connection().await;
        let mut backend = test_backend_with_tabs(connection.clone(), &["target-active"]);
        backend.headful = true;
        backend.display_available = true;
        // Page.bringToFront 发到 page session——先在注册表登记它（生产由 attach 事件登记）。
        connection
            .registry()
            .register_session("session-target-active", "page");

        backend
            .bring_to_front()
            .await
            .expect("foreground command sequence succeeds");

        drop(backend);
        connection.shutdown().await;
        let requests = server.await.expect("fake server joins");
        let methods = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                "Browser.getWindowForTarget",
                "Browser.setWindowBounds",
                "Target.activateTarget",
                "Page.bringToFront",
            ],
            "restore window, select tab, then deliver document focus (F37)"
        );
        assert_eq!(requests[0]["params"]["targetId"], "target-active");
        assert_eq!(requests[1]["params"]["windowId"], 73);
        assert_eq!(requests[1]["params"]["bounds"]["windowState"], "normal");
        assert_eq!(requests[2]["params"]["targetId"], "target-active");
        assert!(
            requests[..3]
                .iter()
                .all(|request| request.get("sessionId").is_none()),
            "Browser and Target window commands must use the root session"
        );
        // F37：Page.bringToFront 必须发在 page session 上（文档焦点属于 renderer）。
        assert_eq!(requests[3]["sessionId"], "session-target-active");
    }

    #[tokio::test]
    async fn close_tab_keeps_visible_state_until_absence_is_proven_and_hands_off_cleanup() {
        let visible = vec![serde_json::json!({
            "targetId": "target-a",
            "type": "page"
        })];
        let (connection, server) = scripted_close_target_fake_connection(
            vec![
                FakeCloseReply::Error(TransportError::Cdp {
                    code: -32000,
                    message: "transient close failure".into(),
                }),
                FakeCloseReply::Success(true),
            ],
            vec![visible],
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);
        let progress = Progress::new(Duration::from_secs(5));

        assert!(backend.close_tab_impl("target-a", &progress).await.is_err());
        assert!(
            backend.tabs.lock().await.contains_key("target-a"),
            "a failed close without absence proof must not create a hidden renderer"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while backend.cleanup_executor.queued_jobs.load(Ordering::Acquire) != 0
                || backend.cleanup_executor.active_jobs.load(Ordering::Acquire) != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("exact cleanup handoff completes through the bounded executor");

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted close fake joins"),
            vec!["Target.closeTarget", "Target.getTargets", "Target.closeTarget"]
        );
    }

    #[tokio::test]
    async fn close_tab_accepts_non_success_response_only_after_exact_absence_inventory() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(false)],
            vec![Vec::new()],
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a", "target-b"]);
        let progress = Progress::new(Duration::from_secs(5));

        backend
            .close_tab_impl("target-b", &progress)
            .await
            .expect("root inventory proves idempotent close");
        assert!(!backend.tabs.lock().await.contains_key("target-b"));
        assert!(backend.tabs.lock().await.contains_key("target-a"));

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted close fake joins"),
            vec!["Target.closeTarget", "Target.getTargets"]
        );
    }

    #[tokio::test]
    async fn shutdown_lane_propagates_close_target_error_and_preserves_state() {
        let (connection, server) = close_target_fake_connection(vec![Err(TransportError::Cdp {
            code: -32000,
            message: "close denied".into(),
        })])
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        let error = backend.shutdown_lane().await.unwrap_err();
        match error {
            BrowserError::Other(message) => assert!(message.contains("close denied"), "{message}"),
            other => panic!("expected propagated CDP error, got {other:?}"),
        }
        assert!(
            backend.lane_closing.load(Ordering::Acquire),
            "failed cleanup still fences new lane work"
        );
        assert!(
            !backend.lane_closed.load(Ordering::Acquire),
            "lane must not publish closed after closeTarget failure"
        );
        assert!(
            backend.tabs.lock().await.contains_key("target-a"),
            "target state must remain for retry"
        );
        assert!(
            backend.lane_close_confirmed.lock().await.is_empty(),
            "failed target must not be marked confirmed"
        );

        drop(backend);
        connection.shutdown().await;
        let closed_targets = server.await.expect("fake server joins");
        assert_eq!(closed_targets, vec!["target-a"]);
    }

    #[tokio::test]
    async fn shutdown_lane_retries_only_unconfirmed_targets_after_partial_success() {
        let (connection, server) = close_target_fake_connection(vec![
            Ok(()),
            Err(TransportError::Cdp {
                code: -32000,
                message: "transient close failure".into(),
            }),
            Ok(()),
        ])
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a", "target-b"]);

        assert!(
            backend.shutdown_lane().await.is_err(),
            "first cleanup should surface the failing second target"
        );
        assert_eq!(
            backend
                .lane_close_confirmed
                .lock()
                .await
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["target-a".to_string()],
            "successful target is remembered so retry is idempotent"
        );
        assert!(
            backend.tabs.lock().await.contains_key("target-a")
                && backend.tabs.lock().await.contains_key("target-b"),
            "tabs remain authoritative until the whole lane closes"
        );

        backend
            .shutdown_lane()
            .await
            .expect("retry closes only the remaining target");
        assert!(backend.lane_closed.load(Ordering::Acquire));
        assert!(backend.tabs.lock().await.is_empty());
        assert!(backend.lane_close_confirmed.lock().await.is_empty());

        drop(backend);
        connection.shutdown().await;
        let closed_targets = server.await.expect("fake server joins");
        assert_eq!(
            closed_targets,
            vec![
                "target-a".to_string(),
                "target-b".to_string(),
                "target-b".to_string()
            ],
            "retry must not send closeTarget again for the already-confirmed target"
        );
    }

    #[tokio::test]
    async fn shutdown_lane_retries_retired_finalizer_without_reentering_active_drain() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(true)],
            vec![
                vec![],
                vec![serde_json::json!({"targetId": "malformed-without-type"})],
                vec![],
                vec![],
            ],
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();
        let mut backend = test_backend_with_tabs(connection.clone(), &["target-a"]);
        router
            .register_lane(
                backend.lane_id.clone(),
                &backend.tabs,
                &backend.active_target,
                &backend.active_frame,
                backend.lane_closing.clone(),
                None,
            )
            .await;
        assert!(router.claim_target(&backend.lane_id, "target-a").await);
        backend.test_router = Some(router.clone());

        let first_error = backend
            .shutdown_lane()
            .await
            .expect_err("malformed first retired inventory must fail closed");
        assert!(
            first_error.to_string().contains("string type"),
            "{first_error}"
        );
        assert!(
            backend.lane_retired.load(Ordering::Acquire),
            "active drain completion must survive retired-finalizer failure"
        );
        assert!(!backend.lane_closed.load(Ordering::Acquire));
        assert!(
            !router
                .state
                .lock()
                .await
                .lanes
                .contains_key(&backend.lane_id),
            "first attempt has already unregistered the active Lane route"
        );

        backend
            .shutdown_lane()
            .await
            .expect("second shutdown retries the retired finalizer directly");
        assert!(backend.lane_closed.load(Ordering::Acquire));

        drop(backend);
        router_loop.abort();
        let _ = router_loop.await;
        connection.shutdown().await;
        let methods = server.await.expect("scripted fake server joins");
        assert_eq!(
            methods,
            vec![
                "Target.closeTarget",
                "Target.getTargets",
                "Target.getTargets",
                "Target.getTargets",
                "Target.getTargets",
            ],
            "retry must not issue another close or consult the removed active Lane"
        );
    }

    #[tokio::test]
    async fn shutdown_lane_accepts_close_error_only_when_inventory_proves_absence() {
        let (connection, server) = close_target_inventory_fake_connection(
            FakeCloseReply::Error(TransportError::Cdp {
                code: -32000,
                message: "No target with given id".into(),
            }),
            Vec::new(),
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        backend
            .shutdown_lane()
            .await
            .expect("an absent target makes close failure idempotent");
        assert!(backend.lane_closed.load(Ordering::Acquire));
        assert!(backend.tabs.lock().await.is_empty());

        drop(backend);
        connection.shutdown().await;
        let methods = server.await.expect("fake server joins");
        assert_eq!(
            methods,
            vec!["Target.closeTarget", "Target.getTargets"],
            "absence must be proven by a root target inventory"
        );
    }

    #[tokio::test]
    async fn shutdown_lane_accepts_success_false_only_when_inventory_proves_absence() {
        let (connection, server) =
            close_target_inventory_fake_connection(FakeCloseReply::Success(false), Vec::new())
                .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        backend
            .shutdown_lane()
            .await
            .expect("success=false is idempotent only for an absent target");
        assert!(backend.lane_closed.load(Ordering::Acquire));

        drop(backend);
        connection.shutdown().await;
        let methods = server.await.expect("fake server joins");
        assert_eq!(methods, vec!["Target.closeTarget", "Target.getTargets"]);
    }

    #[tokio::test]
    async fn shutdown_lane_rejects_success_false_when_target_is_still_present() {
        let (connection, server) =
            close_target_inventory_fake_connection(
                FakeCloseReply::Success(false),
                vec!["target-a"],
            )
            .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        let error = backend.shutdown_lane().await.unwrap_err();
        match error {
            BrowserError::Other(message) => {
                assert!(message.contains("success=false"), "{message}")
            }
            other => panic!("expected raw closeTarget failure, got {other:?}"),
        }
        assert!(!backend.lane_closed.load(Ordering::Acquire));
        assert!(backend.tabs.lock().await.contains_key("target-a"));

        drop(backend);
        connection.shutdown().await;
        let methods = server.await.expect("fake server joins");
        assert_eq!(methods, vec!["Target.closeTarget", "Target.getTargets"]);
    }

    #[tokio::test]
    async fn shutdown_lane_rejects_missing_close_success_when_target_is_present() {
        let (connection, server) = close_target_inventory_fake_connection(
            FakeCloseReply::Raw(serde_json::json!({})),
            vec!["target-a"],
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        let error = backend.shutdown_lane().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("did not contain boolean success=true"),
            "{error}"
        );
        assert!(!backend.lane_closed.load(Ordering::Acquire));

        drop(backend);
        connection.shutdown().await;
        assert_eq!(
            server.await.expect("fake server joins"),
            vec!["Target.closeTarget", "Target.getTargets"]
        );
    }

    #[tokio::test]
    async fn shutdown_lane_rejects_non_boolean_close_success_when_target_is_present() {
        let (connection, server) = close_target_inventory_fake_connection(
            FakeCloseReply::Raw(serde_json::json!({ "success": "yes" })),
            vec!["target-a"],
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        let error = backend.shutdown_lane().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("did not contain boolean success=true"),
            "{error}"
        );
        assert!(!backend.lane_closed.load(Ordering::Acquire));

        drop(backend);
        connection.shutdown().await;
        assert_eq!(
            server.await.expect("fake server joins"),
            vec!["Target.closeTarget", "Target.getTargets"]
        );
    }

    #[tokio::test]
    async fn shutdown_lane_accepts_malformed_close_only_after_valid_absence_inventory() {
        let (connection, server) = close_target_inventory_fake_connection(
            FakeCloseReply::Raw(serde_json::json!({})),
            Vec::new(),
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        backend
            .shutdown_lane()
            .await
            .expect("valid empty inventory proves target absence");

        drop(backend);
        connection.shutdown().await;
        assert_eq!(
            server.await.expect("fake server joins"),
            vec!["Target.closeTarget", "Target.getTargets"]
        );
    }

    #[tokio::test]
    async fn shutdown_lane_rejects_malformed_target_inventory() {
        let (connection, server) = close_target_inventory_fake_connection_with_infos(
            FakeCloseReply::Raw(serde_json::json!({})),
            vec![serde_json::json!({})],
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        let error = backend.shutdown_lane().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("did not contain boolean success=true"),
            "the original close failure must remain authoritative: {error}"
        );
        assert!(!backend.lane_closed.load(Ordering::Acquire));

        drop(backend);
        connection.shutdown().await;
        assert_eq!(
            server.await.expect("fake server joins"),
            vec!["Target.closeTarget", "Target.getTargets"]
        );
    }

    #[tokio::test]
    async fn shutdown_lane_retains_original_close_error_when_target_is_present() {
        let (connection, server) = close_target_inventory_fake_connection(
            FakeCloseReply::Error(TransportError::Cdp {
                code: -32000,
                message: "close denied by policy".into(),
            }),
            vec!["target-a"],
        )
        .await;
        let backend = test_backend_with_tabs(connection.clone(), &["target-a"]);

        let error = backend.shutdown_lane().await.unwrap_err();
        match error {
            BrowserError::Other(message) => {
                assert!(message.contains("close denied by policy"), "{message}")
            }
            other => panic!("expected original CDP error, got {other:?}"),
        }
        assert!(!backend.lane_closed.load(Ordering::Acquire));
        assert!(backend.tabs.lock().await.contains_key("target-a"));
        assert!(backend.lane_close_confirmed.lock().await.is_empty());

        drop(backend);
        connection.shutdown().await;
        let methods = server.await.expect("fake server joins");
        assert_eq!(methods, vec!["Target.closeTarget", "Target.getTargets"]);
    }

    #[tokio::test]
    async fn lane_closing_fences_popup_attach_before_lane_closed() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_target = Arc::new(AsyncMutex::new("opener".to_string()));
        let active_frame = Arc::new(AsyncMutex::new(None));
        let lane_closing = Arc::new(AtomicBool::new(false));
        let lane_closed = Arc::new(AtomicBool::new(false));

        router
            .register_lane(
                "lane-a".to_string(),
                &tabs,
                &active_target,
                &active_frame,
                lane_closing.clone(),
                None,
            )
            .await;
        assert!(router.claim_target("lane-a", "opener").await);

        let attach_router = router.clone();
        let attach = tokio::spawn(async move {
            attach_router
                .handle_attached(PendingPage {
                    target_id: "popup".to_string(),
                    session_id: "popup-session".to_string(),
                    opener_target_id: Some("opener".to_string()),
                    target_url: None,
                })
                .await;
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if router.state.lock().await.ownership.owner("popup") == Some("lane-a") {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("popup attach must enter the router before close starts");

        lane_closing.store(true, Ordering::Release);
        assert!(
            !lane_closed.load(Ordering::Acquire),
            "the router fence must engage before final lane_closed publication"
        );
        tokio::time::timeout(Duration::from_millis(250), attach)
            .await
            .expect("closing must promptly stop an in-flight popup attach")
            .expect("popup attach task must not panic");

        assert!(
            tabs.lock().await.is_empty(),
            "a popup must not enter the Lane after lane_closing is published"
        );
        assert!(
            !router.claim_target("lane-a", "explicit-after-close").await,
            "explicit target claims must also fail once Lane closing begins"
        );
        assert_eq!(
            router
                .state
                .lock()
                .await
                .ownership
                .owner("explicit-after-close"),
            None
        );

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn event_barrier_times_out_when_router_never_acknowledges() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());

        let started = tokio::time::Instant::now();
        let error = router
            .event_barrier_with_timeout(Duration::from_millis(50))
            .await
            .expect_err("an unspawned router cannot acknowledge its mailbox barrier");
        assert!(error.to_string().contains("barrier timed out"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "barrier acknowledgement must have a hard upper bound"
        );

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn event_barrier_times_out_when_router_mailbox_is_full() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        for _ in 0..16 {
            let (ack_tx, _ack_rx) = tokio::sync::oneshot::channel();
            router
                .barrier_tx
                .try_send(ack_tx)
                .expect("test fills the exact mailbox capacity");
        }

        let started = tokio::time::Instant::now();
        let error = router
            .event_barrier_with_timeout(Duration::from_millis(50))
            .await
            .expect_err("a full mailbox must not block Lane cleanup forever");
        assert!(error.to_string().contains("barrier timed out"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "barrier enqueue must share the same hard deadline as acknowledgement"
        );

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn retired_lane_fence_closes_popup_attached_after_unregister_and_preserves_sibling() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![
                FakeCloseReply::Error(TransportError::Cdp {
                    code: -32000,
                    message: "transient close race".into(),
                }),
                FakeCloseReply::Success(true),
            ],
            vec![vec![serde_json::json!({
                "targetId": "late-popup",
                "type": "page",
                "openerId": "retired-opener"
            })]],
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs_a = Arc::new(AsyncMutex::new(HashMap::new()));
        let tabs_b = Arc::new(AsyncMutex::new(HashMap::from([(
            "sibling-target".to_string(),
            test_tab_record(&connection, "sibling-target", "sibling-session"),
        )])));
        let active_a = Arc::new(AsyncMutex::new("retired-opener".to_string()));
        let active_b = Arc::new(AsyncMutex::new("sibling-target".to_string()));
        let frame_a = Arc::new(AsyncMutex::new(None));
        let frame_b = Arc::new(AsyncMutex::new(None));
        let closing_a = Arc::new(AtomicBool::new(true));
        router
            .register_lane(
                "lane-a".into(),
                &tabs_a,
                &active_a,
                &frame_a,
                closing_a,
                None,
            )
            .await;
        router
            .register_lane(
                "lane-b".into(),
                &tabs_b,
                &active_b,
                &frame_b,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await;
        {
            let mut state = router.state.lock().await;
            state
                .ownership
                .claim("lane-a", "retired-opener")
                .unwrap();
            state
                .ownership
                .claim("lane-b", "sibling-target")
                .unwrap();
        }

        // Models the causal gap after a final empty Target.getTargets result:
        // the Lane is already unregistered when its queued popup attach arrives.
        router.unregister_lane("lane-a").await;
        router
            .handle_attached(PendingPage {
                target_id: "late-popup".into(),
                session_id: "late-popup-session".into(),
                opener_target_id: Some("retired-opener".into()),
                target_url: None,
            })
            .await;

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let state = router.state.lock().await;
                let cleaned = !state.quarantined.contains_key("late-popup")
                    && !state.cleanup_inflight.contains("late-popup")
                    && !state
                        .session_targets
                        .values()
                        .any(|target| target == "late-popup");
                drop(state);
                if cleaned {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retired popup cleanup retries until close succeeds");

        {
            let state = router.state.lock().await;
            assert_eq!(
                state.retired_target_owner.get("retired-opener"),
                Some(&"lane-a".to_string())
            );
            assert_eq!(
                state.retired_target_owner.get("late-popup"),
                Some(&"lane-a".to_string())
            );
            assert_eq!(state.ownership.owner("late-popup"), None);
            assert_eq!(state.ownership.owner("sibling-target"), Some("lane-b"));
            assert!(!state.lost_targets.contains_key("late-popup"));
        }
        assert!(tabs_b.lock().await.contains_key("sibling-target"));
        assert_eq!(active_b.lock().await.as_str(), "sibling-target");

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted fake server joins"),
            vec![
                "Target.closeTarget",
                "Target.getTargets",
                "Target.closeTarget"
            ]
        );
    }

    #[tokio::test]
    async fn retired_lane_finalize_waits_for_inflight_close_and_ordered_empty_inventories() {
        let present = || {
            vec![serde_json::json!({
                "targetId": "late-popup",
                "type": "page",
                "openerId": "retired-opener"
            })]
        };
        let (connection, server) = scripted_close_target_fake_connection(
            vec![
                FakeCloseReply::Error(TransportError::Cdp {
                    code: -32000,
                    message: "transient close race".into(),
                }),
                FakeCloseReply::Success(true),
            ],
            vec![present(), present(), Vec::new(), Vec::new()],
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();
        let tabs_a = Arc::new(AsyncMutex::new(HashMap::new()));
        let tabs_b = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_a = Arc::new(AsyncMutex::new("retired-opener".to_string()));
        let active_b = Arc::new(AsyncMutex::new("sibling-target".to_string()));
        let frame_a = Arc::new(AsyncMutex::new(None));
        let frame_b = Arc::new(AsyncMutex::new(None));
        router
            .register_lane(
                "lane-a".into(),
                &tabs_a,
                &active_a,
                &frame_a,
                Arc::new(AtomicBool::new(true)),
                None,
            )
            .await;
        router
            .register_lane(
                "lane-b".into(),
                &tabs_b,
                &active_b,
                &frame_b,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await;
        {
            let mut state = router.state.lock().await;
            state
                .ownership
                .claim("lane-a", "retired-opener")
                .unwrap();
            state
                .ownership
                .claim("lane-b", "sibling-target")
                .unwrap();
        }
        router.unregister_lane("lane-a").await;
        router
            .handle_attached(PendingPage {
                target_id: "late-popup".into(),
                session_id: "late-popup-session".into(),
                opener_target_id: Some("retired-opener".into()),
                target_url: None,
            })
            .await;

        let finalize_router = Arc::clone(&router);
        let finalize = tokio::spawn(async move {
            finalize_router.finalize_retired_lane("lane-a").await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !finalize.is_finished(),
            "Lane finalization must not return while its first close retry is in flight"
        );
        tokio::time::timeout(Duration::from_secs(3), finalize)
            .await
            .expect("finalization completes after cleanup succeeds")
            .expect("finalization task does not panic")
            .expect("ordered inventories prove the retired Lane empty");
        {
            let state = router.state.lock().await;
            assert!(!state.cleanup_inflight.contains("late-popup"));
            assert_eq!(state.ownership.owner("sibling-target"), Some("lane-b"));
        }

        router_loop.abort();
        let _ = router_loop.await;
        connection.shutdown().await;
        let methods = server.await.expect("scripted fake server joins");
        assert_eq!(
            methods
                .iter()
                .filter(|method| method.as_str() == "Target.closeTarget")
                .count(),
            2
        );
        assert_eq!(
            methods
                .iter()
                .filter(|method| method.as_str() == "Target.getTargets")
                .count(),
            4
        );
    }

    #[tokio::test]
    async fn nonce_correlated_create_uses_operation_deadline_not_short_unknown_grace() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let pending_url = "data:text/html,nomi-pending-test";
        let operation_deadline =
            tokio::time::Instant::now() + PENDING_PAGE_CREATE_RECOVERY_TIMEOUT;
        router
            .register_pending_create(pending_url, operation_deadline, None, None)
            .await
            .expect("pending create intent registers");

        let attached_at = tokio::time::Instant::now();
        router
            .handle_attached(PendingPage {
                target_id: "slow-legitimate-create".into(),
                session_id: "slow-legitimate-session".into(),
                opener_target_id: None,
                target_url: Some(pending_url.into()),
            })
            .await;
        let state = router.state.lock().await;
        let cleanup_after = state
            .quarantined
            .get("slow-legitimate-create")
            .and_then(|page| page.cleanup_after)
            .expect("correlated target stays reserved");
        assert!(
            cleanup_after >= attached_at + PENDING_PAGE_CREATE_RECOVERY_TIMEOUT,
            "a valid nonce create gets a fresh full arm deadline after attach"
        );
        assert!(!state
            .pending_create_urls
            .contains_key(pending_url));
        drop(state);

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn nonce_correlated_page_binds_trusted_family_and_worker_inherits_lane() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let pending_url = "about:blank#trusted-family-correlation";
        let scope = TaskTabReservationScope {
            task_resource_key: "trusted-family-a".into(),
            lane_id: "trusted-lane-a".into(),
            authority: Arc::new(RejectingTaskTabAuthority),
        };
        router
            .register_pending_create(
                pending_url,
                tokio::time::Instant::now() + PENDING_PAGE_CREATE_RECOVERY_TIMEOUT,
                None,
                Some(&scope),
            )
            .await
            .expect("trusted pending create intent registers");
        assert_eq!(
            connection.registry().register_attached(
                ROOT_SESSION,
                "trusted-page-session",
                "trusted-page-target",
                "page",
                None,
            ),
            TaskSessionAdmission::PendingAuthority
        );

        router
            .handle_attached(PendingPage {
                target_id: "trusted-page-target".into(),
                session_id: "trusted-page-session".into(),
                opener_target_id: None,
                target_url: Some(pending_url.into()),
            })
            .await;
        assert_eq!(
            connection
                .registry()
                .task_session_authority("trusted-page-session"),
            Some(crate::session::TaskSessionAuthority {
                task_resource_family_key: "trusted-family-a".into(),
                lane_id: "trusted-lane-a".into(),
            })
        );
        assert_eq!(
            connection.registry().register_attached(
                "trusted-page-session",
                "trusted-worker-session",
                "trusted-worker-target",
                "worker",
                None,
            ),
            TaskSessionAdmission::Admitted
        );
        assert_eq!(
            connection
                .registry()
                .task_session_authority("trusted-worker-session"),
            Some(crate::session::TaskSessionAuthority {
                task_resource_family_key: "trusted-family-a".into(),
                lane_id: "trusted-lane-a".into(),
            })
        );

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn unknown_top_level_target_gets_short_grace_then_bounded_exact_cleanup() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(true)],
            Vec::new(),
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();

        router
            .handle_attached(PendingPage {
                target_id: "noopener-popup".into(),
                session_id: "noopener-session".into(),
                opener_target_id: None,
                target_url: None,
            })
            .await;
        {
            let state = router.state.lock().await;
            assert!(state.quarantined.contains_key("noopener-popup"));
            assert!(!state.cleanup_inflight.contains("noopener-popup"));
        }

        tokio::time::timeout(QUARANTINED_TARGET_GRACE + Duration::from_secs(2), async {
            loop {
                let state = router.state.lock().await;
                let cleaned = !state.quarantined.contains_key("noopener-popup")
                    && !state.cleanup_inflight.contains("noopener-popup")
                    && !state
                        .session_targets
                        .values()
                        .any(|target_id| target_id == "noopener-popup");
                drop(state);
                if cleaned {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("unowned target is closed after its bounded claim grace");

        router_loop.abort();
        let _ = router_loop.await;
        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted fake joins"),
            vec!["Target.closeTarget"]
        );
    }

    #[tokio::test]
    async fn lowering_sixteen_to_one_requires_sibling_lane_prune_and_isolates_other_task() {
        let (connection, server) =
            scripted_close_target_fake_connection(Vec::new(), Vec::new()).await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs_a = Arc::new(AsyncMutex::new(
            (0..8)
                .map(|index| {
                    let target = format!("task-a-{index:02}");
                    let session = format!("task-a-session-{index:02}");
                    (target.clone(), test_tab_record(&connection, &target, &session))
                })
                .collect::<HashMap<_, _>>(),
        ));
        let tabs_b = Arc::new(AsyncMutex::new(
            (0..8)
                .map(|index| {
                    let target = format!("task-b-{index:02}");
                    let session = format!("task-b-session-{index:02}");
                    (target.clone(), test_tab_record(&connection, &target, &session))
                })
                .collect::<HashMap<_, _>>(),
        ));
        let tabs_other = Arc::new(AsyncMutex::new(HashMap::from([(
            "other-active".to_string(),
            test_tab_record(&connection, "other-active", "other-session"),
        )])));
        let active_a = Arc::new(AsyncMutex::new("task-a-03".to_string()));
        let active_b = Arc::new(AsyncMutex::new("task-b-04".to_string()));
        let active_other = Arc::new(AsyncMutex::new("other-active".to_string()));
        let frame_a = Arc::new(AsyncMutex::new(None));
        let frame_b = Arc::new(AsyncMutex::new(None));
        let frame_other = Arc::new(AsyncMutex::new(None));

        for (lane_id, tabs, active, frame, task) in [
            ("lane-a", &tabs_a, &active_a, &frame_a, "shared-task"),
            ("lane-b", &tabs_b, &active_b, &frame_b, "shared-task"),
            (
                "other-lane",
                &tabs_other,
                &active_other,
                &frame_other,
                "other-task",
            ),
        ] {
            router
                .register_lane_with_resource_scope(
                    lane_id.into(),
                    tabs,
                    active,
                    frame,
                    Arc::new(AtomicBool::new(false)),
                    None,
                    Some(task.into()),
                    16,
                    None,
                )
                .await
                .expect("test Lane registers");
        }

        let error = router
            .prepare_task_tab_limit_reconciliation("shared-task", 1)
            .await
            .unwrap_err();
        assert!(matches!(error, BrowserError::Blocked { .. }));
        assert_eq!(router.task_tab_limit("shared-task").await, Some(16));
        assert_eq!(router.task_tab_limit("other-task").await, Some(16));

        // Hub policy reconciliation first closes the deterministic excess
        // Lane. The Host can then preserve the remaining Lane's active page
        // and select every other target for exact close.
        router.unregister_lane("lane-b").await;
        let plan = router
            .prepare_task_tab_limit_reconciliation("shared-task", 1)
            .await
            .expect("one surviving Lane can be reconciled to one page");
        assert_eq!(plan.excess_tabs.len(), 1);
        assert_eq!(plan.excess_tabs[0].0, "lane-a");
        assert_eq!(plan.excess_tabs[0].1.len(), 7);
        assert!(!plan.excess_tabs[0].1.iter().any(|id| id == "task-a-03"));
        assert_eq!(router.task_tab_limit("shared-task").await, Some(1));
        assert_eq!(router.task_tab_limit("other-task").await, Some(16));
        assert_eq!(tabs_other.lock().await.len(), 1);

        connection.shutdown().await;
        assert!(server.await.expect("scripted fake joins").is_empty());
    }

    #[tokio::test]
    async fn completed_raise_is_not_relowered_by_a_stale_starting_lane() {
        let (connection, server) = scripted_close_target_fake_connection(Vec::new(), Vec::new()).await;
        let router = HostTargetRouter::new(connection.clone());
        let running_tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "running-target".to_string(),
            test_tab_record(&connection, "running-target", "running-session"),
        )])));
        let running_active = Arc::new(AsyncMutex::new("running-target".to_string()));
        let running_frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane_with_resource_scope(
                "running-lane".into(),
                &running_tabs,
                &running_active,
                &running_frame,
                Arc::new(AtomicBool::new(false)),
                None,
                Some("raised-task".into()),
                4,
                None,
            )
            .await
            .expect("running Lane registers under the old cap");

        router
            .prepare_task_tab_limit_reconciliation("raised-task", 8)
            .await
            .expect("Host route commits the raised cap");
        assert_eq!(router.task_tab_limit("raised-task").await, Some(8));

        let stale_tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "stale-target".to_string(),
            test_tab_record(&connection, "stale-target", "stale-session"),
        )])));
        let stale_active = Arc::new(AsyncMutex::new("stale-target".to_string()));
        let stale_frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane_with_resource_scope(
                "stale-starting-lane".into(),
                &stale_tabs,
                &stale_active,
                &stale_frame,
                Arc::new(AtomicBool::new(false)),
                None,
                Some("raised-task".into()),
                4,
                None,
            )
            .await
            .expect("a Lane already started under the old cap may still publish");

        assert_eq!(
            router.task_tab_limit("raised-task").await,
            Some(8),
            "the stale Lane snapshot must inherit the committed Host policy instead of lowering every sibling route back to four"
        );

        connection.shutdown().await;
        assert!(server.await.expect("scripted fake joins").is_empty());
    }

    #[tokio::test]
    async fn lowering_rejects_a_create_claimed_under_the_old_cap_before_publish() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(true)],
            Vec::new(),
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "existing-active".to_string(),
            test_tab_record(&connection, "existing-active", "existing-session"),
        )])));
        let active = Arc::new(AsyncMutex::new("existing-active".to_string()));
        let frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane_with_resource_scope(
                "race-lane".into(),
                &tabs,
                &active,
                &frame,
                Arc::new(AtomicBool::new(false)),
                None,
                Some("race-task".into()),
                16,
                None,
            )
            .await
            .expect("race Lane registers");
        assert!(router.claim_target("race-lane", "inflight-create").await);

        let plan = router
            .prepare_task_tab_limit_reconciliation("race-task", 1)
            .await
            .expect("existing active page already fits new cap");
        assert!(plan.excess_tabs.is_empty());
        let outcome = router
            .publish_armed_page(
                "race-lane",
                PendingPage {
                    target_id: "inflight-create".into(),
                    session_id: "inflight-session".into(),
                    opener_target_id: None,
                    target_url: None,
                },
                test_tab_record(&connection, "inflight-create", "inflight-session"),
            )
            .await;
        assert_eq!(outcome, OwnedPagePublish::RejectedCapacity);
        assert_eq!(tabs.lock().await.len(), 1);

        tokio::time::timeout(Duration::from_secs(2), async {
            while router
                .state
                .lock()
                .await
                .cleanup_inflight
                .contains("inflight-create")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("old-cap create is closed by bounded exact cleanup");
        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted fake joins"),
            vec!["Target.closeTarget"]
        );
    }

    #[tokio::test]
    async fn task_tab_limit_is_atomic_across_sibling_lanes_and_rejects_popup() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(true)],
            Vec::new(),
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs_a = Arc::new(AsyncMutex::new(HashMap::from([(
            "task-tab-a".to_string(),
            test_tab_record(&connection, "task-tab-a", "task-session-a"),
        )])));
        let tabs_b = Arc::new(AsyncMutex::new(HashMap::new()));
        let active_a = Arc::new(AsyncMutex::new("task-tab-a".to_string()));
        let active_b = Arc::new(AsyncMutex::new(String::new()));
        let frame_a = Arc::new(AsyncMutex::new(None));
        let frame_b = Arc::new(AsyncMutex::new(None));
        router
            .register_lane_with_resource_scope(
                "task-lane-a".into(),
                &tabs_a,
                &active_a,
                &frame_a,
                Arc::new(AtomicBool::new(false)),
                None,
                Some("shared-task".into()),
                1,
                None,
            )
            .await
            .expect("first task Lane registers");
        router
            .register_lane_with_resource_scope(
                "task-lane-b".into(),
                &tabs_b,
                &active_b,
                &frame_b,
                Arc::new(AtomicBool::new(false)),
                None,
                Some("shared-task".into()),
                16,
                None,
            )
            .await
            .expect("empty sibling Lane registers at the same task cap");
        assert_eq!(
            router
                .state
                .lock()
                .await
                .lanes
                .get("task-lane-b")
                .expect("sibling route exists")
                .max_task_tabs,
            1,
            "a stale higher Lane launch snapshot must persist the live stricter task cap"
        );
        assert!(router.claim_target("task-lane-a", "task-tab-a").await);
        assert!(router.claim_target("task-lane-b", "task-popup-b").await);

        let outcome = router
            .publish_armed_page(
                "task-lane-b",
                PendingPage {
                    target_id: "task-popup-b".into(),
                    session_id: "task-popup-session-b".into(),
                    opener_target_id: Some("task-tab-a".into()),
                    target_url: None,
                },
                test_tab_record(&connection, "task-popup-b", "task-popup-session-b"),
            )
            .await;
        assert_eq!(outcome, OwnedPagePublish::RejectedCapacity);
        assert!(tabs_b.lock().await.is_empty());

        tokio::time::timeout(Duration::from_secs(2), async {
            while router
                .state
                .lock()
                .await
                .cleanup_inflight
                .contains("task-popup-b")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("rejected task popup cleanup completes");
        assert_eq!(tabs_a.lock().await.len() + tabs_b.lock().await.len(), 1);

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted fake joins"),
            vec!["Target.closeTarget"]
        );
    }

    #[tokio::test]
    async fn task_tab_permit_outlives_ui_record_until_exact_absence_proof() {
        let (connection, server) = scripted_close_target_fake_connection(Vec::new(), Vec::new()).await;
        let router = HostTargetRouter::new(connection.clone());
        let drops = Arc::new(AtomicUsize::new(0));
        let reservation: Arc<dyn TaskTabReservation> = Arc::new(CountingTaskTabReservation {
            drops: Arc::clone(&drops),
        });
        let mut record = test_tab_record(&connection, "permit-target", "permit-session");
        record._task_tab_reservation = Some(Arc::clone(&reservation));
        let tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "permit-target".to_string(),
            record,
        )])));
        router
            .state
            .lock()
            .await
            .target_tab_reservations
            .insert("permit-target".into(), Arc::clone(&reservation));
        drop(reservation);

        let record = tabs
            .lock()
            .await
            .remove("permit-target")
            .expect("UI registry owns the test target");
        abort_tab_record(&record);
        drop(record);
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "removing a UI TabRecord must not release physical task capacity"
        );

        router.release_target("permit-target", None).await;
        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "the final permit releases only after exact target absence proof"
        );

        connection.shutdown().await;
        assert!(server.await.expect("scripted fake joins").is_empty());
    }

    #[tokio::test]
    async fn cancelled_initial_lane_releases_exact_permit_without_destroyed_event() {
        let (connection, server) = scripted_close_target_fake_connection(
            vec![FakeCloseReply::Success(true)],
            Vec::new(),
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let drops = Arc::new(AtomicUsize::new(0));
        let reservation: Arc<dyn TaskTabReservation> = Arc::new(CountingTaskTabReservation {
            drops: Arc::clone(&drops),
        });
        let mut record = test_tab_record(&connection, "cancelled-target", "cancelled-session");
        record._task_tab_reservation = Some(Arc::clone(&reservation));
        let tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "cancelled-target".to_string(),
            record,
        )])));
        {
            let mut state = router.state.lock().await;
            state
                .target_tab_reservations
                .insert("cancelled-target".into(), Arc::clone(&reservation));
            state
                .session_targets
                .insert("cancelled-session".into(), "cancelled-target".into());
        }
        let cleanup = LaneCleanupAuthority::new(
            connection.clone(),
            Arc::clone(&router.cleanup_executor),
            Arc::clone(&router),
            "cancelled-lane".into(),
            Arc::new(AtomicBool::new(true)),
            Arc::clone(&tabs),
            "cancelled-target".into(),
            "cancelled-session".into(),
            "cancelled-frame".into(),
            Some(&reservation),
            None,
        );
        drop(reservation);

        Arc::clone(&cleanup).finish().await;

        assert_eq!(cleanup.state.load(Ordering::Acquire), 2);
        assert!(tabs.lock().await.is_empty());
        assert!(!router
            .state
            .lock()
            .await
            .target_tab_reservations
            .contains_key("cancelled-target"));
        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "a successful exact close releases the router permit even when Chromium emits no targetDestroyed event"
        );

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("scripted fake joins"),
            vec!["Target.closeTarget"]
        );
    }

    #[tokio::test]
    async fn repeated_cancelled_initial_lanes_converge_single_slot_task_capacity() {
        const ITERATIONS: usize = 32;
        let (connection, server) = scripted_close_target_fake_connection(
            (0..ITERATIONS)
                .map(|_| FakeCloseReply::Success(true))
                .collect(),
            Vec::new(),
        )
        .await;
        let router = HostTargetRouter::new(connection.clone());
        let live = Arc::new(AtomicUsize::new(0));
        let authority = BoundedTaskTabAuthority {
            live: Arc::clone(&live),
            max: 1,
        };

        for index in 0..ITERATIONS {
            let target_id = format!("cancelled-target-{index}");
            let session_id = format!("cancelled-session-{index}");
            let frame_id = format!("cancelled-frame-{index}");
            let lane_id = format!("cancelled-lane-{index}");
            let reservation = authority
                .reserve("one-slot-task", &lane_id, &target_id)
                .await
                .expect("the previous cancelled Lane returned its sole task slot");
            assert_eq!(live.load(Ordering::SeqCst), 1);

            let mut record = test_tab_record(&connection, &target_id, &session_id);
            record._task_tab_reservation = Some(Arc::clone(&reservation));
            let tabs = Arc::new(AsyncMutex::new(HashMap::from([(
                target_id.clone(),
                record,
            )])));
            router
                .state
                .lock()
                .await
                .target_tab_reservations
                .insert(target_id.clone(), Arc::clone(&reservation));
            let cleanup = LaneCleanupAuthority::new(
                connection.clone(),
                Arc::clone(&router.cleanup_executor),
                Arc::clone(&router),
                lane_id,
                Arc::new(AtomicBool::new(true)),
                tabs,
                target_id.clone(),
                session_id,
                frame_id,
                Some(&reservation),
                None,
            );
            drop(reservation);

            cleanup.finish().await;

            assert_eq!(
                live.load(Ordering::SeqCst),
                0,
                "cancelled initial Lane {index} leaked the sole task tab slot"
            );
            assert!(!router
                .state
                .lock()
                .await
                .target_tab_reservations
                .contains_key(&target_id));
        }

        connection.shutdown().await;
        let methods = server.await.expect("scripted fake joins");
        assert_eq!(methods.len(), ITERATIONS);
        assert!(methods.iter().all(|method| method == "Target.closeTarget"));
    }

    #[tokio::test]
    async fn late_cancelled_lane_scrub_preserves_replacement_reservation() {
        let (connection, server) = scripted_close_target_fake_connection(Vec::new(), Vec::new()).await;
        let router = HostTargetRouter::new(connection.clone());
        let old_drops = Arc::new(AtomicUsize::new(0));
        let new_drops = Arc::new(AtomicUsize::new(0));
        let old_reservation: Arc<dyn TaskTabReservation> = Arc::new(CountingTaskTabReservation {
            drops: Arc::clone(&old_drops),
        });
        let new_reservation: Arc<dyn TaskTabReservation> = Arc::new(CountingTaskTabReservation {
            drops: Arc::clone(&new_drops),
        });
        router
            .state
            .lock()
            .await
            .target_tab_reservations
            .insert("reused-key".into(), Arc::clone(&new_reservation));

        router
            .scrub_cancelled_lane_target(
                "old-lane",
                "reused-key",
                "old-session",
                "old-frame",
                true,
                Some(&old_reservation),
            )
            .await;

        let retained = router
            .state
            .lock()
            .await
            .target_tab_reservations
            .get("reused-key")
            .cloned()
            .expect("late old-generation cleanup preserves the replacement permit");
        assert!(Arc::ptr_eq(&retained, &new_reservation));
        drop(retained);
        drop(old_reservation);
        assert_eq!(old_drops.load(Ordering::SeqCst), 1);
        assert_eq!(new_drops.load(Ordering::SeqCst), 0);

        router.release_target("reused-key", None).await;
        drop(new_reservation);
        assert_eq!(new_drops.load(Ordering::SeqCst), 1);
        connection.shutdown().await;
        assert!(server.await.expect("scripted fake joins").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quarantine_overflow_is_bounded_and_escalates_whole_host_cleanup() {
        let (connection, mut methods, server) = saturated_cleanup_fake_connection().await;
        let router = HostTargetRouter::new(connection.clone());

        for index in 0..=MAX_QUARANTINED_TARGETS {
            router
                .handle_attached(PendingPage {
                    target_id: format!("unowned-{index}"),
                    session_id: format!("unowned-session-{index}"),
                    opener_target_id: None,
                    target_url: None,
                })
                .await;
        }
        {
            let state = router.state.lock().await;
            assert_eq!(state.quarantined.len(), MAX_QUARANTINED_TARGETS);
            assert!(state.session_targets.len() <= MAX_QUARANTINED_TARGETS);
            assert!(state.cleanup_inflight.len() <= MAX_ROUTER_CLEANUP_INFLIGHT);
        }
        assert!(router.cleanup_executor.is_poisoned());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("overflow escalates promptly"),
            Some("Browser.close".into())
        );

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("overflow fake joins"),
            vec!["Browser.close"]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn popup_attach_overflow_is_bounded_per_lane_and_escalates_host_cleanup() {
        let (connection, mut methods, server) = saturated_cleanup_fake_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
        let active = Arc::new(AsyncMutex::new("owned-0".to_string()));
        let frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane(
                "bounded-lane".into(),
                &tabs,
                &active,
                &frame,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await;
        {
            let mut state = router.state.lock().await;
            for index in 0..MAX_TRACKED_TARGETS_PER_LANE {
                state
                    .ownership
                    .claim("bounded-lane", format!("owned-{index}"))
                    .unwrap();
            }
        }

        router
            .handle_attached(PendingPage {
                target_id: "excess-popup".into(),
                session_id: "excess-popup-session".into(),
                opener_target_id: Some("owned-0".into()),
                target_url: None,
            })
            .await;

        {
            let state = router.state.lock().await;
            assert_eq!(
                state.active_lane_target_count("bounded-lane"),
                MAX_TRACKED_TARGETS_PER_LANE
            );
            assert_eq!(state.ownership.owner("excess-popup"), None);
            assert!(!state.session_targets.contains_key("excess-popup-session"));
        }
        assert!(router.cleanup_executor.is_poisoned());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("per-Lane overflow escalates promptly"),
            Some("Browser.close".into())
        );

        connection.shutdown().await;
        assert_eq!(
            server.await.expect("per-Lane overflow fake joins"),
            vec!["Browser.close"]
        );
    }

    #[tokio::test]
    async fn active_lane_loss_tombstones_are_reclaimed_on_every_unregister() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());

        for round in 0..8 {
            let lane_id = format!("lane-{round}");
            let target_id = format!("target-{round}");
            let tabs = Arc::new(AsyncMutex::new(HashMap::new()));
            let active = Arc::new(AsyncMutex::new(target_id.clone()));
            let frame = Arc::new(AsyncMutex::new(None));
            router
                .register_lane(
                    lane_id.clone(),
                    &tabs,
                    &active,
                    &frame,
                    Arc::new(AtomicBool::new(false)),
                    None,
                )
                .await;
            assert!(router.claim_target(&lane_id, &target_id).await);
            router
                .handle_top_level_target_loss(
                    Some(target_id.clone()),
                    None,
                    TopLevelTargetLoss::Destroyed,
                )
                .await;
            assert!(router
                .state
                .lock()
                .await
                .lost_targets
                .contains_key(&target_id));
            router.unregister_lane(&lane_id).await;
            assert!(
                router.state.lock().await.lost_targets.is_empty(),
                "active Lane tombstones must not accumulate across close rounds"
            );
        }

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn destroyed_target_tombstone_suppresses_late_session_loss_cleanup() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        {
            let mut state = router.state.lock().await;
            state
                .ownership
                .claim("late-event-lane", "already-destroyed")
                .expect("test target has one owner");
            state.lost_targets.insert(
                "already-destroyed".into(),
                tokio::time::Instant::now() + LOST_TARGET_TOMBSTONE_GRACE,
            );
        }

        router
            .handle_top_level_session_loss(
                Some("already-destroyed".into()),
                None,
                TopLevelTargetLoss::Detached,
            )
            .await;

        assert!(
            router.state.lock().await.cleanup_inflight.is_empty(),
            "a queued detach after physical destruction must not mint a cleanup worker"
        );

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_target_retains_task_slot_until_exact_cleanup_and_coalesces_repeats() {
        let (connection, events, mut methods, allow_exact_close, server) =
            detached_target_cleanup_fake_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let router_loop = router.spawn();
        let live = Arc::new(AtomicUsize::new(0));
        let authority = BoundedTaskTabAuthority {
            live: Arc::clone(&live),
            max: 1,
        };
        let reservation = authority
            .reserve("detach-task", "detach-lane", "detached-target")
            .await
            .expect("initial target reserves the sole task slot");
        let mut record = test_tab_record(&connection, "detached-target", "detached-session");
        record._task_tab_reservation = Some(Arc::clone(&reservation));
        let tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "detached-target".to_string(),
            record,
        )])));
        let active = Arc::new(AsyncMutex::new("detached-target".to_string()));
        let frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane_with_resource_scope(
                "detach-lane".into(),
                &tabs,
                &active,
                &frame,
                Arc::new(AtomicBool::new(false)),
                None,
                Some("detach-task".into()),
                1,
                None,
            )
            .await
            .expect("detach test Lane registers");
        assert!(router.claim_target("detach-lane", "detached-target").await);
        {
            let mut state = router.state.lock().await;
            state
                .session_targets
                .insert("detached-session".into(), "detached-target".into());
            state
                .target_tab_reservations
                .insert("detached-target".into(), Arc::clone(&reservation));
        }
        drop(reservation);

        let detach_event = serde_json::json!({
            "method": "Target.detachedFromTarget",
            "params": {
                "sessionId": "detached-session",
                "targetId": "detached-target"
            }
        });
        events
            .send(detach_event.clone())
            .expect("fake sends first detach");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("detach starts exact target close"),
            Some("Target.closeTarget".into())
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("failed close checks root inventory"),
            Some("Target.getTargets".into())
        );

        assert_eq!(live.load(Ordering::SeqCst), 1);
        assert!(tabs.lock().await.contains_key("detached-target"));
        assert!(router
            .state
            .lock()
            .await
            .target_tab_reservations
            .contains_key("detached-target"));
        assert!(matches!(
            authority
                .reserve("detach-task", "second-lane", "must-be-rejected")
                .await,
            Err(BrowserError::Blocked { .. })
        ));

        events
            .send(detach_event)
            .expect("fake sends repeated detach");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), methods.recv())
                .await
                .expect("the single cleanup worker retries"),
            Some("Target.closeTarget".into())
        );
        allow_exact_close.notify_one();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let converged = live.load(Ordering::SeqCst) == 0
                    && tabs.lock().await.is_empty()
                    && !router
                        .state
                        .lock()
                        .await
                        .target_tab_reservations
                        .contains_key("detached-target");
                if converged {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("exact close releases the physical task slot");
        let replacement = authority
            .reserve("detach-task", "replacement-lane", "replacement-target")
            .await
            .expect("capacity is reusable only after exact target cleanup");
        drop(replacement);
        assert_eq!(live.load(Ordering::SeqCst), 0);

        connection.shutdown().await;
        router_loop.abort();
        drop(events);
        assert_eq!(
            server.await.expect("detached-target fake joins"),
            vec!["Target.closeTarget", "Target.getTargets", "Target.closeTarget"]
        );
    }

    #[tokio::test]
    async fn top_level_destroyed_updates_only_the_owning_lane() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs_a = Arc::new(AsyncMutex::new(HashMap::from([(
            "target-a".to_string(),
            test_tab_record(&connection, "target-a", "page-session-a"),
        )])));
        let tabs_b = Arc::new(AsyncMutex::new(HashMap::from([(
            "target-b".to_string(),
            test_tab_record(&connection, "target-b", "page-session-b"),
        )])));
        let active_a = Arc::new(AsyncMutex::new("target-a".to_string()));
        let active_b = Arc::new(AsyncMutex::new("target-b".to_string()));
        let frame_a = Arc::new(AsyncMutex::new(Some((
            "target-a".to_string(),
            "frame-a".to_string(),
        ))));
        let frame_b = Arc::new(AsyncMutex::new(Some((
            "target-b".to_string(),
            "frame-b".to_string(),
        ))));
        router
            .register_lane(
                "lane-a".into(),
                &tabs_a,
                &active_a,
                &frame_a,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await;
        router
            .register_lane(
                "lane-b".into(),
                &tabs_b,
                &active_b,
                &frame_b,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await;
        assert!(router.claim_target("lane-a", "target-a").await);
        assert!(router.claim_target("lane-b", "target-b").await);
        {
            let mut state = router.state.lock().await;
            state
                .session_targets
                .insert("page-session-a".into(), "target-a".into());
            state
                .session_targets
                .insert("page-session-b".into(), "target-b".into());
        }

        router
            .handle_top_level_target_loss(
                Some("target-a".into()),
                None,
                TopLevelTargetLoss::Destroyed,
            )
            .await;
        assert!(tabs_a.lock().await.is_empty());
        assert_eq!(active_a.lock().await.as_str(), "");
        assert!(frame_a.lock().await.is_none());
        assert!(tabs_b.lock().await.contains_key("target-b"));
        assert_eq!(active_b.lock().await.as_str(), "target-b");
        assert!(
            router.state.lock().await.ownership.owner("target-a") == Some("lane-a"),
            "lost target ownership remains a cleanup tombstone"
        );

        router
            .handle_top_level_target_loss(
                Some("target-b".into()),
                None,
                TopLevelTargetLoss::Destroyed,
            )
            .await;
        assert!(tabs_b.lock().await.is_empty());
        assert_eq!(active_b.lock().await.as_str(), "");
        assert!(frame_b.lock().await.is_none());

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn worker_and_oopif_loss_cannot_remove_top_level_lane_tabs() {
        let (connection, server) = router_test_connection().await;
        let router = HostTargetRouter::new(connection.clone());
        let tabs = Arc::new(AsyncMutex::new(HashMap::from([(
            "page-target".to_string(),
            test_tab_record(&connection, "page-target", "page-session"),
        )])));
        let active = Arc::new(AsyncMutex::new("page-target".to_string()));
        let frame = Arc::new(AsyncMutex::new(None));
        router
            .register_lane(
                "lane-a".into(),
                &tabs,
                &active,
                &frame,
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await;
        assert!(router.claim_target("lane-a", "page-target").await);
        router
            .state
            .lock()
            .await
            .session_targets
            .insert("page-session".into(), "page-target".into());

        router
            .handle_top_level_session_loss(
                None,
                Some("worker-session".into()),
                TopLevelTargetLoss::Detached,
            )
            .await;
        router
            .handle_top_level_target_loss(
                Some("oopif-target".into()),
                None,
                TopLevelTargetLoss::Destroyed,
            )
            .await;

        assert!(tabs.lock().await.contains_key("page-target"));
        assert_eq!(active.lock().await.as_str(), "page-target");
        assert!(!router.is_target_lost("oopif-target").await);

        server.abort();
        let _ = server.await;
        connection.shutdown().await;
    }

    // ── B6 detach/crash 事件源接线：事件 params 形状判定（[纯逻辑]，喂构造 Value，无浏览器）──

    #[test]
    fn event_session_matches_by_session_id() {
        // Target.detachedFromTarget 的 params.sessionId 命中本 page session → true。
        let p = serde_json::json!({"sessionId": "PAGE_SID", "targetId": "T1"});
        assert!(event_session_matches(&p, "PAGE_SID"));
        // 不同 session（其它 target 没了）→ false（不误 abort 本动作）。
        assert!(!event_session_matches(&p, "OTHER_SID"));
        // 缺 sessionId 字段 → false（保守不 abort）。
        let p2 = serde_json::json!({"targetId": "T1"});
        assert!(!event_session_matches(&p2, "PAGE_SID"));
        // sessionId 非字符串（坏形状）→ false。
        let p3 = serde_json::json!({"sessionId": 7});
        assert!(!event_session_matches(&p3, "PAGE_SID"));
    }

    #[test]
    fn target_crash_matches_target_id_not_root_session() {
        let crash = serde_json::json!({
            "targetId": "TARGET_A",
            "status": "crashed",
            "errorCode": 1
        });
        assert!(event_target_matches(&crash, "TARGET_A"));
        assert!(!event_target_matches(&crash, "TARGET_B"));
        assert!(
            !event_session_matches(&crash, "PAGE_SESSION"),
            "root targetCrashed has no child sessionId"
        );
        assert!(!event_target_matches(
            &serde_json::json!({"sessionId": "PAGE_SESSION"}),
            "TARGET_A"
        ));
    }

    #[test]
    fn crashed_active_target_selects_a_deterministic_survivor() {
        let first = ["target-z", "target-a", "target-m"];
        let second = ["target-m", "target-z", "target-a"];
        assert_eq!(
            deterministic_survivor(first, "target-z").as_deref(),
            Some("target-a")
        );
        assert_eq!(
            deterministic_survivor(second, "target-z").as_deref(),
            Some("target-a"),
            "HashMap/event order must not change survivor selection"
        );
        assert_eq!(deterministic_survivor(["only"], "only"), None);
    }

    #[test]
    fn event_frame_matches_by_frame_id() {
        // Page.frameDetached 的 params.frameId 命中动作所在帧 → true。
        let p = serde_json::json!({"frameId": "FRAME_A", "reason": "remove"});
        assert!(event_frame_matches(&p, "FRAME_A"));
        // 不同帧（无关子帧 detach）→ false（不误 abort 本动作）。
        assert!(!event_frame_matches(&p, "FRAME_B"));
        // 缺 frameId → false。
        let p2 = serde_json::json!({"reason": "remove"});
        assert!(!event_frame_matches(&p2, "FRAME_A"));
        // frameId 非字符串 → false。
        let p3 = serde_json::json!({"frameId": null});
        assert!(!event_frame_matches(&p3, "FRAME_A"));
    }

    #[test]
    fn transport_timeout_maps_to_nav_failed() {
        let e = map_transport_err(TransportError::Timeout);
        assert!(matches!(e, BrowserError::NavFailed { .. }));
    }

    #[test]
    fn transport_connection_closed_maps_to_host_session_lost() {
        assert!(matches!(
            map_transport_err(TransportError::Closed),
            BrowserError::SessionLost { recoverable: false }
        ));
    }

    #[test]
    fn transport_session_closed_stays_target_local() {
        assert!(matches!(
            map_transport_err(TransportError::SessionClosed),
            BrowserError::TargetClosed
        ));
    }

    #[test]
    fn transport_session_crashed_stays_target_local() {
        assert!(matches!(
            map_transport_err(TransportError::SessionCrashed),
            BrowserError::TargetCrashed
        ));
    }

    #[test]
    fn transport_cdp_error_maps_to_other_with_code() {
        let e = map_transport_err(TransportError::Cdp {
            code: -32000,
            message: "Cannot find context".into(),
        });
        match e {
            BrowserError::Other(msg) => {
                assert!(msg.contains("-32000"), "msg: {msg}");
                assert!(msg.contains("Cannot find context"), "msg: {msg}");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn transport_protocol_maps_to_other() {
        let e = map_transport_err(TransportError::Protocol("bad".into()));
        assert!(matches!(e, BrowserError::Other(_)));
    }

    #[test]
    fn map_inject_err_classifies_all_variants() {
        // JsException → Other（保留原文）。
        match map_inject_err(InjectError::JsException("boom".into())) {
            BrowserError::Other(msg) => assert_eq!(msg, "boom"),
            other => panic!("expected Other, got {other:?}"),
        }
        // Protocol → Other（保留原文）。
        match map_inject_err(InjectError::Protocol("weird shape".into())) {
            BrowserError::Other(msg) => assert_eq!(msg, "weird shape"),
            other => panic!("expected Other, got {other:?}"),
        }
        // ContextNotReady → NavFailed{kind:"context"}（与 frame_id 无关，恒定 kind）。
        match map_inject_err(InjectError::ContextNotReady {
            frame_id: "F0".into(),
        }) {
            BrowserError::NavFailed { kind } => assert_eq!(kind, "context"),
            other => panic!("expected NavFailed, got {other:?}"),
        }
        match map_inject_err(InjectError::ContextCapacityExceeded { limit: 256 }) {
            BrowserError::Blocked { reason } => {
                assert!(reason.contains("256"));
                assert!(reason.contains("frame tree"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        match map_inject_err(InjectError::RefCapacityExceeded {
            limit: 2048,
            current: 2050,
            required: 25,
        }) {
            BrowserError::Blocked { reason } => {
                assert!(reason.contains("2048"));
                assert!(reason.contains("fresh observe"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        // Transport(Timeout) 经 map_transport_err 复用 → NavFailed。
        assert!(matches!(
            map_inject_err(InjectError::Transport(TransportError::Timeout)),
            BrowserError::NavFailed { .. }
        ));
        // Transport(Closed) 复用 → SessionLost{recoverable:false}（确认确实走 map_transport_err 全语义）。
        assert!(matches!(
            map_inject_err(InjectError::Transport(TransportError::Closed)),
            BrowserError::SessionLost { recoverable: false }
        ));
    }

    #[test]
    fn base64_roundtrip_png_magic() {
        // "PNG" 的 base64 是 "UE5H"；解码应得回 b"PNG"。
        assert_eq!(decode_base64("UE5H"), Some(b"PNG".to_vec()));
    }

    #[test]
    fn base64_handles_padding() {
        // "hi" → "aGk="（含填充）。CDP 截图 data 是干净标准 base64，无需容忍内嵌空白。
        assert_eq!(decode_base64("aGk="), Some(b"hi".to_vec()));
    }

    #[test]
    fn base64_rejects_invalid_char() {
        assert!(decode_base64("not base64 !@#").is_none());
    }

    #[test]
    fn base64_real_png_header() {
        // 真 PNG 文件头 8 字节: 137 80 78 71 13 10 26 10 → base64 "iVBORw0KGgo="。
        let decoded = decode_base64("iVBORw0KGgo=").unwrap();
        assert_eq!(&decoded[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        // 注意：&decoded[1..4] == b"PNG"（冒烟断言用的就是这个切片）。
        assert_eq!(&decoded[1..4], b"PNG");
    }

    // ── observe 纯逻辑 helper（无浏览器）：ref/role/name 解析 + 递归缝合 + depth 粗判 ──

    #[test]
    fn parse_ref_token_extracts_ref_only() {
        assert_eq!(
            parse_ref_token(r#"  - button "Submit order" [ref=f0e1]"#).as_deref(),
            Some("f0e1")
        );
        // [cursor=pointer] 后缀不混入 ref。
        assert_eq!(
            parse_ref_token("  - link \"X\" [ref=f2e9] [cursor=pointer]").as_deref(),
            Some("f2e9")
        );
        // 无 ref 行 → None。
        assert_eq!(parse_ref_token("  - text: hello"), None);
    }

    #[test]
    fn parse_seq_from_ref_extracts_frame_seq() {
        assert_eq!(parse_seq_from_ref("f0e1"), Some(0));
        assert_eq!(parse_seq_from_ref("f12e345"), Some(12));
        assert_eq!(parse_seq_from_ref("bogus"), None);
        assert_eq!(parse_seq_from_ref("fXe1"), None);
    }

    #[test]
    fn parse_role_name_splits_role_and_quoted_name() {
        assert_eq!(
            parse_role_name(r#"  - button "Submit order" [ref=f0e1]"#),
            ("button".to_string(), "Submit order".to_string())
        );
        // 无 name（如 iframe / generic）→ name 空。
        assert_eq!(
            parse_role_name("  - iframe [ref=f0e5]"),
            ("iframe".to_string(), String::new())
        );
        // 带属性标记的 role 仍只取 role token。
        assert_eq!(
            parse_role_name(r#"- checkbox "Remember me" [checked] [ref=f0e3]"#),
            ("checkbox".to_string(), "Remember me".to_string())
        );
    }

    #[test]
    fn observe_ref_parser_rejects_oversized_generation_atomically() {
        let frames = vec![ObservedFrame {
            seq: 0,
            frame_id: "MAIN".into(),
            session_id: "SESSION".into(),
            snapshot: FrameSnapshot {
                full: String::new(),
                incremental: None,
                iframe_refs: vec![],
                iframe_depths: HashMap::new(),
            },
        }];
        let stitched = (0..=MAX_REFS_PER_GENERATION)
            .map(|index| format!("- button \"{index}\" [ref=f0e{index}]"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut table = RefTable::new_generation(None);
        let error = CdpBackend::parse_refs_into_table(&frames, &stitched, &mut table)
            .expect_err("one observe generation may not publish more than the hard bound");
        assert_eq!(
            error,
            RefTableCapacityError {
                limit: MAX_REFS_PER_GENERATION
            }
        );
        assert!(
            table.is_empty(),
            "parser overflow must not leave a partially resolvable generation"
        );
    }

    #[test]
    fn observe_ref_parser_never_publishes_unknown_frame_refs() {
        let frames = vec![ObservedFrame {
            seq: 0,
            frame_id: "MAIN".into(),
            session_id: "SESSION".into(),
            snapshot: FrameSnapshot {
                full: String::new(),
                incremental: None,
                iframe_refs: vec![],
                iframe_depths: HashMap::new(),
            },
        }];
        let mut table = RefTable::new_generation(None);
        let entries = CdpBackend::parse_refs_into_table(
            &frames,
            "- button \"real\" [ref=f0e1]\n- button \"spoof\" [ref=f99e1]",
            &mut table,
        )
        .expect("small generation fits");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].r#ref, "f0e1");
        assert!(table.resolve("f0e1").is_some());
        assert!(table.resolve("f99e1").is_none());
    }

    #[test]
    fn extract_quoted_respects_escapes() {
        assert_eq!(extract_quoted(r#"a "he said \"hi\"" b"#).as_deref(), Some("he said \"hi\""));
        assert_eq!(extract_quoted("no quotes here"), None);
    }

    #[test]
    fn frame_hit_depth_limit_detects_deep_indent() {
        let shallow = FrameSnapshot {
            full: "- button \"X\" [ref=f0e1]".into(),
            incremental: None,
            iframe_refs: vec![],
            iframe_depths: std::collections::HashMap::new(),
        };
        assert!(!frame_hit_depth_limit(&shallow, 12));
        // 一行缩进达 24 空格 = depth 12 层 → 触顶。
        let deep = FrameSnapshot {
            full: format!("{}- text: deep", " ".repeat(24)),
            incremental: None,
            iframe_refs: vec![],
            iframe_depths: std::collections::HashMap::new(),
        };
        assert!(frame_hit_depth_limit(&deep, 12));
        // max_depth=0 = 不封顶 → 恒 false。
        assert!(!frame_hit_depth_limit(&deep, 0));
    }

    #[test]
    fn render_frame_recursive_stitches_nested_frames() {
        // 三帧树：f0(主) → f1(子, 经 ref f0e5) → f2(孙, 经 ref f1e3)。
        let frames = vec![
            ObservedFrame {
                seq: 0,
                frame_id: "MAIN".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: "- generic:\n  - iframe [ref=f0e5]".into(),
                    incremental: None,
                    iframe_refs: vec!["f0e5".into()],
                    iframe_depths: std::collections::HashMap::from([("f0e5".to_string(), 1u32)]),
                },
            },
            ObservedFrame {
                seq: 1,
                frame_id: "CHILD".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: "- iframe [ref=f1e3]".into(),
                    incremental: None,
                    iframe_refs: vec!["f1e3".into()],
                    iframe_depths: std::collections::HashMap::from([("f1e3".to_string(), 0u32)]),
                },
            },
            ObservedFrame {
                seq: 2,
                frame_id: "GRAND".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: "- link \"Deep\" [ref=f2e1]".into(),
                    incremental: None,
                    iframe_refs: vec![],
                    iframe_depths: std::collections::HashMap::new(),
                },
            },
        ];
        let parent_of = HashMap::from([
            ("CHILD".to_string(), ("MAIN".to_string(), "f0e5".to_string())),
            ("GRAND".to_string(), ("CHILD".to_string(), "f1e3".to_string())),
        ]);
        let out = render_frame_recursive_bounded(&frames, 0, &parent_of)
            .expect("small nested frame tree fits the observation budget");
        // 主帧 iframe 行内联子帧，子帧 iframe 行再内联孙帧。
        assert!(out.contains("- iframe [ref=f0e5]:"), "out:\n{out}");
        assert!(out.contains("f1e3]:"), "child iframe should be opened:\n{out}");
        assert!(out.contains("Deep"), "grandchild content missing:\n{out}");
        // 孙内容缩进应比子内容更深。
        let deep_line = out.lines().find(|l| l.contains("Deep")).unwrap();
        let child_line = out.lines().find(|l| l.contains("f1e3]")).unwrap();
        let deep_indent = deep_line.len() - deep_line.trim_start().len();
        let child_indent = child_line.len() - child_line.trim_start().len();
        assert!(deep_indent > child_indent, "grandchild not deeper than child");
    }

    #[test]
    fn render_frame_recursive_rejects_deep_iframe_indentation_at_byte_cap() {
        // A hostile frame can claim a giant aria depth.  The renderer must
        // reject before allocating the corresponding indentation String.
        let frames = vec![
            ObservedFrame {
                seq: 0,
                frame_id: "MAIN".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: "- iframe [ref=f0e1]".into(),
                    incremental: None,
                    iframe_refs: vec!["f0e1".into()],
                    iframe_depths: HashMap::from([(
                        "f0e1".to_string(),
                        (MAX_OBSERVATION_RETAINED_BYTES as u32),
                    )]),
                },
            },
            ObservedFrame {
                seq: 1,
                frame_id: "CHILD".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: "- button \"child\" [ref=f1e1]".into(),
                    incremental: None,
                    iframe_refs: vec![],
                    iframe_depths: HashMap::new(),
                },
            },
        ];
        let parent_of = HashMap::from([(
            "CHILD".to_string(),
            ("MAIN".to_string(), "f0e1".to_string()),
        )]);
        let error = render_frame_recursive_bounded(&frames, 0, &parent_of)
            .expect_err("indentation expansion beyond 4 MiB must fail closed");
        assert_eq!(error.limit, MAX_OBSERVATION_RETAINED_BYTES);
    }

    #[test]
    fn render_frame_recursive_rejects_multi_frame_aggregate_at_byte_cap() {
        let half = MAX_OBSERVATION_RETAINED_BYTES / 2;
        let frames = vec![
            ObservedFrame {
                seq: 0,
                frame_id: "MAIN".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: format!("- iframe [ref=f0e1]\n{}", "a".repeat(half)),
                    incremental: None,
                    iframe_refs: vec!["f0e1".into()],
                    iframe_depths: HashMap::from([("f0e1".to_string(), 0)]),
                },
            },
            ObservedFrame {
                seq: 1,
                frame_id: "CHILD".into(),
                session_id: "S".into(),
                snapshot: FrameSnapshot {
                    full: "b".repeat(half),
                    incremental: None,
                    iframe_refs: vec![],
                    iframe_depths: HashMap::new(),
                },
            },
        ];
        let parent_of = HashMap::from([(
            "CHILD".to_string(),
            ("MAIN".to_string(), "f0e1".to_string()),
        )]);
        assert!(
            render_frame_recursive_bounded(&frames, 0, &parent_of).is_err(),
            "the aggregate iframe output, not each frame independently, owns the 4 MiB cap"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // F-actions：download 落盘探测 + save_as_pdf 输出路径（[纯逻辑] + 真 FS temp dir，无浏览器）。
    // ═══════════════════════════════════════════════════════════════════════

    /// 给本测建一个唯一临时目录（按测试名 + pid 去歧义，避免并行测试互踩）。
    fn unique_tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nomifun-facts-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn list_dir_files_collects_filenames_and_skips_subdirs() {
        let dir = unique_tmp_dir("listdir");
        // 清场（防上次残留干扰）。
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::write(dir.join("b.pdf"), b"y").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let set = list_dir_files(dir.to_str().unwrap()).expect("small directory snapshot");
        assert!(set.contains("a.txt"));
        assert!(set.contains("b.pdf"));
        assert!(!set.contains("sub"), "subdirectories must not be listed");
        // 不存在的目录 → 空集（best-effort，不 panic）。
        assert!(
            list_dir_files("/no/such/dir/zzz-nonexistent")
                .expect("missing directory remains best-effort empty")
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_completed_download_finds_new_nonempty_nontemp_file() {
        let dir = unique_tmp_dir("newdl");
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let dir_s = dir.to_str().unwrap();
        // 基线：触发前已有 old.txt（不应被当作本次下载）。
        std::fs::write(dir.join("old.txt"), b"old").unwrap();
        let before = list_dir_files(dir_s).expect("small baseline snapshot");
        assert!(before.contains("old.txt"));

        // 新增一个非空、非临时的文件 → 命中。
        std::fs::write(dir.join("report.pdf"), b"%PDF-1.4 some bytes").unwrap();
        let found = newest_completed_download(dir_s, &before)
            .expect("small completion snapshot");
        assert!(found.is_some(), "a new non-empty file must be detected");
        let (name, size) = found.unwrap();
        assert_eq!(name, "report.pdf");
        assert!(size > 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_completed_download_skips_crdownload_empty_and_preexisting() {
        let dir = unique_tmp_dir("skipdl");
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let dir_s = dir.to_str().unwrap();
        let before = list_dir_files(dir_s).expect("empty baseline snapshot");

        // chrome 下载中间态（仍在传）→ 不算完成，跳过。
        std::fs::write(dir.join("inflight.crdownload"), b"partial").unwrap();
        // 0 字节（刚创建占位）→ 跳过。
        std::fs::write(dir.join("placeholder.bin"), b"").unwrap();
        assert!(
            newest_completed_download(dir_s, &before)
                .expect("small completion snapshot")
                .is_none(),
            "only .crdownload + empty files present → no completed download"
        );

        // 触发前已存在的文件（即便非空）→ 不算本次（在 before 集里）。
        let before2 = {
            std::fs::write(dir.join("preexisting.pdf"), b"old-but-nonempty").unwrap();
            list_dir_files(dir_s).expect("small baseline snapshot")
        };
        assert!(
            newest_completed_download(dir_s, &before2)
                .expect("small completion snapshot")
                .is_none(),
            "a file already in the baseline must not be counted as this download"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_directory_snapshot_rejects_n_plus_one_entries_before_allocating_more() {
        let dir = unique_tmp_dir("download-dir-n-plus-one");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create bounded snapshot fixture");
        for index in 0..=MAX_DOWNLOAD_DIRECTORY_SNAPSHOT_ENTRIES {
            std::fs::write(dir.join(format!("entry-{index:04}")), b"x")
                .expect("create bounded snapshot entry");
        }
        let dir_s = dir.to_str().expect("test path is UTF-8");
        assert!(
            matches!(list_dir_files(dir_s), Err(BrowserError::Blocked { .. })),
            "the before snapshot must fail closed at N+1"
        );
        assert!(
            matches!(
                newest_completed_download(dir_s, &HashSet::new()),
                Err(BrowserError::Blocked { .. })
            ),
            "the after snapshot must scan the bounded inventory before returning a match"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_directory_snapshot_accounts_single_and_total_name_bytes() {
        let mut total = 0;
        account_download_directory_name(&mut total, MAX_DOWNLOAD_DIRECTORY_SINGLE_NAME_BYTES)
            .expect("single name at the limit is accepted");
        assert!(matches!(
            account_download_directory_name(
                &mut total,
                MAX_DOWNLOAD_DIRECTORY_SINGLE_NAME_BYTES + 1
            ),
            Err(BrowserError::Blocked { .. })
        ));

        let mut total = MAX_DOWNLOAD_DIRECTORY_NAME_BYTES;
        assert!(matches!(
            account_download_directory_name(&mut total, 1),
            Err(BrowserError::Blocked { .. })
        ));
    }

    #[test]
    fn pdf_output_path_is_under_downloads_with_pdf_extension() {
        let path = pdf_output_path("/some/companion/workspace/downloads");
        assert!(path.starts_with("/some/companion/workspace/downloads"));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("page-"), "filename: {name}");
        assert!(name.ends_with(".pdf"), "filename: {name}");
    }
}

/// **op_mutex 序列化纪律（纯逻辑钉死）**：镜像 [`CdpBackend::op_mutex`] 的 acquire 模式（一把
/// `AsyncMutex<()>` 跨整个操作体持有），无需启动 Chrome 即证明两操作不交错。真 Chrome 的 observe⊥act
/// 交错冒烟见 `tests/op_mutex_concurrency.rs`（`#[ignore]`，需 `NOMIFUN_CHROME_BINARY`）。
#[cfg(test)]
mod op_mutex_tests {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex as AsyncMutex;

    #[tokio::test]
    async fn op_mutex_serializes_two_operations() {
        let op_mutex = Arc::new(AsyncMutex::new(()));
        let order = Arc::new(AsyncMutex::new(Vec::<&'static str>::new()));

        // 操作 A：抢到 op_mutex 后跨 await 持有一段时间。
        let (m1, o1) = (op_mutex.clone(), order.clone());
        let a = tokio::spawn(async move {
            let _g = m1.lock().await;
            o1.lock().await.push("a-start");
            tokio::time::sleep(Duration::from_millis(20)).await;
            o1.lock().await.push("a-end");
        });
        // 让 A 先抢到锁。
        tokio::time::sleep(Duration::from_millis(5)).await;
        // 操作 B：必须等 A 整段结束才能拿到锁。
        let (m2, o2) = (op_mutex.clone(), order.clone());
        let b = tokio::spawn(async move {
            let _g = m2.lock().await;
            o2.lock().await.push("b-start");
            o2.lock().await.push("b-end");
        });

        a.await.unwrap();
        b.await.unwrap();
        // B 不得在 A 结束前开始 → observe⊥act 不交错。
        let seen = order.lock().await.clone();
        assert_eq!(
            seen,
            vec!["a-start", "a-end", "b-start", "b-end"],
            "op_mutex 必须串行（无交错）: {seen:?}"
        );
    }
}
