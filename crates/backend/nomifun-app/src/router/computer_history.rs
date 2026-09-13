//! Computer-history REST routes for the settings UI (`ipcBridge.computerHistory`).
//!
//! The UI talks REST for every domain (existing convention); the gateway
//! `computer_history_*` capabilities registered separately serve the agent /
//! Remote surface. Both faces wrap the SAME `ComputerHistoryService`, so the
//! settings page, the gateway and the agent's `computer_history_*` tools can
//! never disagree about recorder state.
//!
//! Response field names here are load-bearing: `ui/src/common/adapter/ipcBridge.ts`
//! types them (`IComputerHistoryStatus` / `IComputerHistorySegment` /
//! `IComputerHistoryAppUsageRow`). Change them in pairs.
//!
//! Route naming note: the set-enabled path deliberately avoids the substring
//! `/enabled` (`/api/computer-history/settings` with a body), matching the
//! agent-wire bridge guard.

use axum::extract::Query;
use axum::Json;
use nomifun_api_types::ApiResponse;
use nomifun_computer_history::ComputerHistoryService;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// Router state: the one service handle, shared with the gateway domain and
/// the agent sink. `None` when the feature-local store failed to open this
/// boot — every handler then answers with the degraded-empty shape so the
/// settings page renders "unavailable" instead of a 5xx.
#[derive(Clone)]
pub(super) struct ComputerHistoryState {
    service: Option<Arc<ComputerHistoryService>>,
}

impl ComputerHistoryState {
    pub(super) fn new(service: Option<Arc<ComputerHistoryService>>) -> Self {
        Self { service }
    }
}

/// GET + DELETE + POST routes, auth-gated by the caller (`routes.rs`).
pub(super) fn computer_history_routes(state: ComputerHistoryState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/computer-history/status", get(status))
        .route("/api/computer-history/segments", get(segments).delete(purge))
        .route("/api/computer-history/app-usage", get(app_usage))
        .route("/api/computer-history/settings", post(set_enabled))
        .with_state(state)
}

// ── response DTOs (field names = UI contract, do not rename) ─────────────

#[derive(Serialize)]
pub(super) struct StorageStatusDto {
    segments: u64,
    approx_bytes: u64,
    path: String,
}

#[derive(Serialize)]
pub(super) struct ComputerHistoryStatusDto {
    enabled: bool,
    /// `stopped | running | paused`.
    state: &'static str,
    /// `granted | denied | unknown`.
    permission: &'static str,
    paused_until: Option<String>,
    storage: StorageStatusDto,
}

#[derive(Serialize)]
pub(super) struct SegmentDto {
    event_id: String,
    app_name: String,
    window_title: Option<String>,
    browser_url: Option<String>,
    started_at_ms: i64,
    ended_at_ms: i64,
    source: String,
}

#[derive(Serialize)]
pub(super) struct AppUsageRowDto {
    app_name: String,
    total_ms: i64,
    segment_count: i64,
}

#[derive(Deserialize)]
pub(super) struct WindowQuery {
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
pub(super) struct PurgeQuery {
    before_ms: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct SetEnabledBody {
    enabled: bool,
}

// ── wire mapping (shared with the gateway domain) ────────────────────────

fn state_wire(state: nomifun_computer_history::RecorderState) -> &'static str {
    use nomifun_computer_history::RecorderState as S;
    match state {
        S::Stopped => "stopped",
        S::Running => "running",
        S::Paused => "paused",
    }
}

fn permission_wire(permission: nomifun_computer_history::PermissionState) -> &'static str {
    use nomifun_computer_history::PermissionState as P;
    match permission {
        P::Granted | P::NotRequired => "granted",
        P::Denied => "denied",
        P::Unknown => "unknown",
    }
}

fn paused_until_wire(
    status: &nomifun_computer_history::ServiceStatus,
) -> Option<String> {
    use chrono::TimeZone;
    status
        .paused_until_ms
        .and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single())
        .map(|dt| dt.to_rfc3339())
}

// ── handlers ─────────────────────────────────────────────────────────────

/// GET /api/computer-history/status
pub(super) async fn status(
    axum::extract::State(state): axum::extract::State<ComputerHistoryState>,
) -> Json<ApiResponse<ComputerHistoryStatusDto>> {
    let Some(service) = state.service else {
        return Json(ApiResponse::ok(ComputerHistoryStatusDto {
            enabled: false,
            state: "stopped",
            permission: "unknown",
            paused_until: None,
            storage: StorageStatusDto {
                segments: 0,
                approx_bytes: 0,
                path: String::new(),
            },
        }));
    };
    match service.status().await {
        Ok(status) => Json(ApiResponse::ok(ComputerHistoryStatusDto {
            enabled: status.enabled,
            state: state_wire(status.state),
            permission: permission_wire(status.permission),
            paused_until: paused_until_wire(&status),
            storage: StorageStatusDto {
                segments: status.storage.segment_count,
                approx_bytes: status.storage.db_bytes + status.storage.segments_dir_bytes,
                path: status.event_stream_root_path,
            },
        })),
        Err(error) => {
            tracing::warn!(%error, "computer history status failed");
            Json(ApiResponse::ok(ComputerHistoryStatusDto {
                enabled: false,
                state: "stopped",
                permission: "unknown",
                paused_until: None,
                storage: StorageStatusDto {
                    segments: 0,
                    approx_bytes: 0,
                    path: String::new(),
                },
            }))
        }
    }
}

/// GET /api/computer-history/segments?from_ms&to_ms&limit
pub(super) async fn segments(
    axum::extract::State(state): axum::extract::State<ComputerHistoryState>,
    Query(query): Query<WindowQuery>,
) -> Json<ApiResponse<Vec<SegmentDto>>> {
    let Some(service) = state.service else {
        return Json(ApiResponse::ok(Vec::new()));
    };
    let filter = nomifun_computer_history::SegmentFilter {
        from_ms: query.from_ms,
        to_ms: query.to_ms,
        app_name: None,
        url_contains: None,
        limit: query.limit.unwrap_or(0),
    };
    let rows = match service.store().query_segments(&filter).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "computer history segments query failed");
            return Json(ApiResponse::ok(Vec::new()));
        }
    };
    Json(ApiResponse::ok(
        rows.into_iter()
            .map(|segment| SegmentDto {
                event_id: segment.event_id,
                app_name: segment.app_name,
                window_title: segment.window_title,
                browser_url: segment.browser_url,
                started_at_ms: segment.started_at_ms,
                ended_at_ms: segment.ended_at_ms,
                source: segment.source,
            })
            .collect(),
    ))
}

/// GET /api/computer-history/app-usage?from_ms&to_ms&limit
pub(super) async fn app_usage(
    axum::extract::State(state): axum::extract::State<ComputerHistoryState>,
    Query(query): Query<WindowQuery>,
) -> Json<ApiResponse<Vec<AppUsageRowDto>>> {
    let Some(service) = state.service else {
        return Json(ApiResponse::ok(Vec::new()));
    };
    let filter = nomifun_computer_history::SegmentFilter {
        from_ms: query.from_ms,
        to_ms: query.to_ms,
        app_name: None,
        url_contains: None,
        limit: query.limit.unwrap_or(0),
    };
    match service.store().app_usage(&filter).await {
        Ok(rows) => Json(ApiResponse::ok(
            rows.into_iter()
                .map(|row| AppUsageRowDto {
                    app_name: row.app_name,
                    total_ms: row.total_ms,
                    segment_count: row.segment_count,
                })
                .collect(),
        )),
        Err(error) => {
            tracing::warn!(%error, "computer history app-usage query failed");
            Json(ApiResponse::ok(Vec::new()))
        }
    }
}

/// POST /api/computer-history/settings — body `{ enabled }`. (Path avoids the
/// `/enabled` substring per the agent-wire bridge guard.)
pub(super) async fn set_enabled(
    axum::extract::State(state): axum::extract::State<ComputerHistoryState>,
    Json(body): Json<SetEnabledBody>,
) -> Json<ApiResponse<serde_json::Value>> {
    let Some(service) = state.service else {
        return Json(ApiResponse::ok(json!({ "ok": false })));
    };
    match service.set_enabled(body.enabled).await {
        Ok(()) => Json(ApiResponse::ok(json!({ "ok": true }))),
        Err(error) => {
            tracing::warn!(%error, "computer history set-enabled failed");
            Json(ApiResponse::ok(json!({ "ok": false })))
        }
    }
}

/// DELETE /api/computer-history/segments?before_ms — purge (whole history when
/// `before_ms` is omitted).
pub(super) async fn purge(
    axum::extract::State(state): axum::extract::State<ComputerHistoryState>,
    Query(query): Query<PurgeQuery>,
) -> Json<ApiResponse<serde_json::Value>> {
    let Some(service) = state.service else {
        return Json(ApiResponse::ok(json!({ "deleted": 0 })));
    };
    let result = match query.before_ms {
        Some(before_ms) => {
            let now = nomifun_common::now_ms();
            service.store().purge_before(before_ms.min(now)).await
        }
        None => service.store().purge_all().await,
    };
    match result {
        Ok(deleted) => Json(ApiResponse::ok(json!({ "deleted": deleted }))),
        Err(error) => {
            tracing::warn!(%error, "computer history purge failed");
            Json(ApiResponse::ok(json!({ "deleted": 0 })))
        }
    }
}
