//! Authenticated browser-platform management routes.
//!
//! This module is a projection over the process-wide `BrowserSessionHub`. It
//! never launches Chromium and never accepts caller/owner identity from the
//! request body. The authenticated `CurrentUser` plus the Hub's trusted lane
//! inventory are the only authority used by these handlers.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_browser_platform::{
    BrowserCapacitySnapshot, BrowserErrorCode, BrowserHostId, BrowserIdentityMode,
    BrowserLaneId, BrowserLaneSnapshot, BrowserOverview, BrowserPlatformError,
    BrowserSessionHub, BrowserSurface, BrowserTabSnapshot, CloseResult,
    BrowserVisibility, HostLifecycleState, LaneLifecycleState, QueueMetadata,
    ResourcePolicy, ResourcePolicyPreset, ResourcePressureState,
    MAX_ACTIVE_OPERATIONS, MAX_BROWSER_MEMORY_RATIO, MAX_GLOBAL_QUEUE, MAX_OPEN_LANES,
    MAX_OWNER_QUEUE, MAX_RESERVED_MEMORY_BYTES, MIN_BROWSER_MEMORY_RATIO,
    MIN_RESERVED_MEMORY_BYTES,
};
use nomifun_db::IClientPreferenceRepository;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[path = "browser_url_projection.rs"]
pub(super) mod browser_url_projection;

use browser_url_projection::project_renderer_url;
use crate::services::{
    BROWSER_DISPLAY_MODE_POLICY_VERSION, BROWSER_DISPLAY_MODE_PREF_KEY,
    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
};

const RESOURCE_POLICY_PREF_KEY: &str = "browser.resourcePolicy";

#[derive(Clone)]
pub struct BrowserManagementState {
    pub hub: Option<Arc<BrowserSessionHub>>,
    pub preferences: Arc<dyn IClientPreferenceRepository>,
    installation_owner_user_id: Arc<str>,
    display_mode_update_gate: Arc<tokio::sync::Mutex<()>>,
}

impl BrowserManagementState {
    pub fn new(
        hub: Option<Arc<BrowserSessionHub>>,
        preferences: Arc<dyn IClientPreferenceRepository>,
        installation_owner_user_id: Arc<str>,
    ) -> Self {
        Self {
            hub,
            preferences,
            installation_owner_user_id,
            display_mode_update_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn require_hub(&self) -> Result<Arc<BrowserSessionHub>, BrowserApiError> {
        self.hub.clone().ok_or_else(BrowserApiError::unsupported)
    }

    fn is_installation_owner(&self, user: &CurrentUser) -> bool {
        user.id.as_str() == self.installation_owner_user_id.as_ref()
    }
}

/// User-scoped Browser management routes.
///
/// Authentication supplies the authoritative user id. Every inventory and
/// Lane mutation handler below scopes itself to that id and deliberately
/// returns not-found for another user's Lane.
pub fn browser_management_user_routes(state: BrowserManagementState) -> Router {
    Router::new()
        .route("/api/browser/overview", get(get_overview))
        .route("/api/browser/lanes", get(get_lanes))
        .route("/api/browser/lanes/{lane_id}/close", post(close_lane))
        .route(
            "/api/browser/lanes/{lane_id}/foreground",
            post(foreground_lane),
        )
        .route(
            "/api/browser/lanes/{lane_id}/background",
            post(background_lane),
        )
        .route(
            "/api/browser/conversations/{conversation_id}/close",
            post(close_conversation),
        )
        .with_state(state)
}

/// Installation-wide Browser controls.
///
/// These routes affect shared process policy or perform a broad lifecycle
/// action, so the top-level router must apply the installation-owner gate in
/// addition to normal authentication.
pub fn browser_management_owner_routes(state: BrowserManagementState) -> Router {
    Router::new()
        .route("/api/browser/close-all", post(close_all))
        .route(
            "/api/browser/resource-policy",
            get(get_resource_policy).put(put_resource_policy),
        )
        .route(
            "/api/browser/display-mode",
            get(get_display_mode).put(put_display_mode),
        )
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct BrowserApiErrorBody {
    code: Value,
    message: String,
    retryable: bool,
    next_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    lane_id: Option<BrowserLaneId>,
    #[serde(skip_serializing_if = "Value::is_null")]
    metadata: Value,
}

#[derive(Debug)]
pub struct BrowserApiError {
    status: StatusCode,
    body: BrowserApiErrorBody,
}

impl BrowserApiError {
    fn unsupported() -> Self {
        Self {
            status: StatusCode::NOT_IMPLEMENTED,
            body: BrowserApiErrorBody {
                code: json!("browser_not_supported"),
                message: "Browser management is not available in this application build.".to_owned(),
                retryable: false,
                next_action: "Use a browser-enabled desktop build.".to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: BrowserApiErrorBody {
                code: json!("invalid_browser_resource_policy"),
                message: message.into(),
                retryable: false,
                next_action: "Correct the browser resource policy and retry.".to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }

    fn invalid_conversation_id(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: BrowserApiErrorBody {
                code: json!("invalid_conversation_id"),
                message: message.into(),
                retryable: false,
                next_action: "Provide a valid conversation id and retry.".to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }

    fn invalid_browser_lane_id() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: BrowserApiErrorBody {
                code: json!("invalid_lane_id"),
                message: "The browser lane id is invalid.".to_owned(),
                retryable: false,
                next_action: "Refresh the browser inventory and retry with a current lane id."
                    .to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }

    fn invalid_display_mode(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: BrowserApiErrorBody {
                code: json!("invalid_browser_display_mode"),
                message: message.into(),
                retryable: false,
                next_action: "Choose either headless or external display mode and retry.".to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }

    fn not_found() -> Self {
        let mut error = Self::from(BrowserPlatformError::lane_not_found(
            BrowserLaneId::parse("unknown").expect("static lane id is valid"),
        ));
        error.body.lane_id = None;
        error
    }

    fn storage() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: BrowserApiErrorBody {
                code: json!("browser_resource_policy_storage_failed"),
                message: "The browser resource policy could not be saved.".to_owned(),
                retryable: true,
                next_action: "Retry the request. If it continues to fail, inspect application storage."
                    .to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }

    fn display_mode_storage() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: BrowserApiErrorBody {
                code: json!("browser_display_mode_storage_failed"),
                message: "The browser display mode could not be saved.".to_owned(),
                retryable: true,
                next_action:
                    "Retry the request. If it continues to fail, inspect application storage."
                        .to_owned(),
                lane_id: None,
                metadata: Value::Null,
            },
        }
    }
}

impl From<BrowserPlatformError> for BrowserApiError {
    fn from(error: BrowserPlatformError) -> Self {
        let metadata = project_browser_error_metadata(&error.metadata);
        let status = match error.code {
            BrowserErrorCode::InvalidCallerIdentity | BrowserErrorCode::InvalidLaneName => {
                StatusCode::BAD_REQUEST
            }
            BrowserErrorCode::OperationNotAllowed | BrowserErrorCode::OwnerLeaseExpired => {
                StatusCode::FORBIDDEN
            }
            BrowserErrorCode::LaneNotFound => StatusCode::NOT_FOUND,
            BrowserErrorCode::BrowserCapacityQueued | BrowserErrorCode::SystemMemoryPressure => {
                StatusCode::TOO_MANY_REQUESTS
            }
            BrowserErrorCode::LaneClosedByUser
            | BrowserErrorCode::StaleBrowserEpoch
            | BrowserErrorCode::StaleLaneRef
            | BrowserErrorCode::TargetCrashed
            | BrowserErrorCode::IdentityReplicaStale
            | BrowserErrorCode::NeedsPrimaryIdentity => StatusCode::CONFLICT,
            BrowserErrorCode::BrowserRestarted
            | BrowserErrorCode::BrowserUnavailable
            | BrowserErrorCode::BrowserShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
        };
        Self {
            status,
            body: BrowserApiErrorBody {
                code: serde_json::to_value(error.code)
                    .unwrap_or_else(|_| json!("browser_unavailable")),
                message: error.message,
                retryable: error.retryable,
                next_action: error.next_action,
                lane_id: error.lane_id,
                metadata,
            },
        }
    }
}

const MAX_BROWSER_ERROR_METADATA_DEPTH: usize = 8;

#[derive(Clone, Copy)]
enum BrowserErrorMetadataScope {
    Root,
    Capacity,
    Queue,
    Recovery,
}

#[derive(Clone, Copy)]
enum BrowserErrorMetadataField {
    Object(BrowserErrorMetadataScope),
    Boolean,
    UnsignedInteger,
    NullableUnsignedInteger,
    ReasonCode,
    PressureState,
    QueueState,
    GenerationRelation,
}

/// Project platform metadata into the narrow renderer-safe Browser API
/// contract. BrowserPlatformError metadata is intentionally extensible inside
/// the platform, so this boundary must fail closed: only capacity, queue, and
/// recovery fields with their expected JSON types are copied.
fn project_browser_error_metadata(metadata: &Value) -> Value {
    project_browser_error_metadata_value(metadata, BrowserErrorMetadataScope::Root, 0)
        .unwrap_or(Value::Null)
}

fn project_browser_error_metadata_value(
    value: &Value,
    scope: BrowserErrorMetadataScope,
    depth: usize,
) -> Option<Value> {
    if depth > MAX_BROWSER_ERROR_METADATA_DEPTH {
        return None;
    }
    let source = value.as_object()?;
    let mut projected = Map::new();

    for (key, value) in source {
        let Some(field) = browser_error_metadata_field(scope, key) else {
            continue;
        };
        let safe_value = match field {
            BrowserErrorMetadataField::Object(next_scope) => {
                project_browser_error_metadata_value(value, next_scope, depth + 1)
            }
            BrowserErrorMetadataField::Boolean => value.as_bool().map(Value::Bool),
            BrowserErrorMetadataField::UnsignedInteger => {
                value.as_u64().map(|_| value.clone())
            }
            BrowserErrorMetadataField::NullableUnsignedInteger => {
                if value.is_null() {
                    Some(Value::Null)
                } else {
                    value.as_u64().map(|_| value.clone())
                }
            }
            BrowserErrorMetadataField::ReasonCode => value
                .as_str()
                .filter(|value| is_safe_browser_reason_code(value))
                .map(|_| value.clone()),
            BrowserErrorMetadataField::PressureState => value
                .as_str()
                .filter(|value| matches!(*value, "normal" | "pressured" | "critical"))
                .map(|_| value.clone()),
            BrowserErrorMetadataField::QueueState => value
                .as_str()
                .filter(|value| matches!(*value, "queued" | "active"))
                .map(|_| value.clone()),
            BrowserErrorMetadataField::GenerationRelation => value
                .as_str()
                .filter(|value| matches!(*value, "older" | "newer"))
                .map(|_| value.clone()),
        };
        if let Some(safe_value) = safe_value {
            projected.insert(key.clone(), safe_value);
        }
    }

    (!projected.is_empty()).then_some(Value::Object(projected))
}

fn browser_error_metadata_field(
    scope: BrowserErrorMetadataScope,
    key: &str,
) -> Option<BrowserErrorMetadataField> {
    match key {
        "capacity" => {
            return Some(BrowserErrorMetadataField::Object(
                BrowserErrorMetadataScope::Capacity,
            ));
        }
        "queue" => {
            return Some(BrowserErrorMetadataField::Object(
                BrowserErrorMetadataScope::Queue,
            ));
        }
        "recovery" => {
            return Some(BrowserErrorMetadataField::Object(
                BrowserErrorMetadataScope::Recovery,
            ));
        }
        _ => {}
    }

    match scope {
        BrowserErrorMetadataScope::Root => root_metadata_field(key),
        BrowserErrorMetadataScope::Capacity => capacity_metadata_field(key),
        BrowserErrorMetadataScope::Queue => queue_metadata_field(key),
        BrowserErrorMetadataScope::Recovery => recovery_metadata_field(key),
    }
}

fn root_metadata_field(key: &str) -> Option<BrowserErrorMetadataField> {
    capacity_metadata_field(key)
        .or_else(|| queue_metadata_field(key))
        .or_else(|| recovery_metadata_field(key))
}

fn capacity_metadata_field(key: &str) -> Option<BrowserErrorMetadataField> {
    match key {
        "active"
        | "queued"
        | "max_active"
        | "max_open_lanes"
        | "recommended_concurrency" => Some(BrowserErrorMetadataField::UnsignedInteger),
        "reason_code" => Some(BrowserErrorMetadataField::ReasonCode),
        "pressure_state" => Some(BrowserErrorMetadataField::PressureState),
        _ => None,
    }
}

fn queue_metadata_field(key: &str) -> Option<BrowserErrorMetadataField> {
    match key {
        "position"
        | "retry_delay_ms"
        | "retry_after_ms"
        | "recommended_concurrency"
        | "owner_active"
        | "owner_queued"
        | "global_active"
        | "global_queued" => Some(BrowserErrorMetadataField::UnsignedInteger),
        "reason_code" => Some(BrowserErrorMetadataField::ReasonCode),
        "request_state" => Some(BrowserErrorMetadataField::QueueState),
        _ => None,
    }
}

fn recovery_metadata_field(key: &str) -> Option<BrowserErrorMetadataField> {
    match key {
        "circuit_open"
        | "fresh_observe_required"
        | "restart_in_progress"
        | "snapshot_available"
        | "refresh_required"
        | "cleanup_pending"
        | "cleanup_task_failed"
        | "task_cancelled"
        | "task_panicked"
        | "start_task_failed"
        | "host_open_lane_task_failed"
        | "host_retired"
        | "lane_not_ready"
        | "closed" => Some(BrowserErrorMetadataField::Boolean),
        "failures_in_window"
        | "failures_remaining"
        | "retry_at_ms"
        | "retry_after_ms"
        | "old_epoch"
        | "new_epoch"
        | "operation_epoch"
        | "current_epoch"
        | "requested_generation"
        | "snapshot_issued_at_ms"
        | "detached_closed"
        | "timeout_ms" => Some(BrowserErrorMetadataField::UnsignedInteger),
        "current_generation" => Some(BrowserErrorMetadataField::NullableUnsignedInteger),
        "generation_relation" => Some(BrowserErrorMetadataField::GenerationRelation),
        _ => None,
    }
}

fn is_safe_browser_reason_code(value: &str) -> bool {
    matches!(
        value,
        "browser_capacity_queued"
            | "browser_resource_pressure"
            | "system_memory_pressure"
            | "global_queue_limit"
            | "owner_queue_limit"
    )
}

trait PolicyUpdateOutcome {
    fn into_policy_result(self) -> Result<(), BrowserPlatformError>;
}

impl PolicyUpdateOutcome for () {
    fn into_policy_result(self) -> Result<(), BrowserPlatformError> {
        Ok(())
    }
}

impl PolicyUpdateOutcome for Result<(), BrowserPlatformError> {
    fn into_policy_result(self) -> Result<(), BrowserPlatformError> {
        self
    }
}

fn policy_update_result(
    outcome: impl PolicyUpdateOutcome,
) -> Result<(), BrowserPlatformError> {
    outcome.into_policy_result()
}

impl IntoResponse for BrowserApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Serialize)]
struct BrowserOverviewDto {
    supported: bool,
    enabled: bool,
    can_close_all: bool,
    can_manage_browser_settings: bool,
    can_manage_primary_identity: bool,
    running_lanes: usize,
    queued_lanes: usize,
    total_lanes: usize,
    managed_host_count: usize,
    pending_cleanup_count: usize,
    pressure_state: ResourcePressureState,
    capacity: BrowserCapacityDto,
    hosts: Vec<BrowserHostDto>,
    updated_at: u64,
}

#[derive(Debug, Serialize)]
struct BrowserCapacityDto {
    active: usize,
    queued: usize,
    max_active: usize,
    max_open_lanes: usize,
    recommended_concurrency: usize,
    reason_code: Option<String>,
}

#[derive(Debug, Serialize)]
struct BrowserHostDto {
    host_id: BrowserHostId,
    state: HostLifecycleState,
    epoch: u64,
    headful: bool,
    identity_mode: BrowserIdentityMode,
    lane_count: usize,
    rss_bytes: Option<u64>,
}

impl From<BrowserCapacitySnapshot> for BrowserCapacityDto {
    fn from(value: BrowserCapacitySnapshot) -> Self {
        Self {
            active: value.active,
            queued: value.queued,
            max_active: value.max_active,
            max_open_lanes: value.max_open_lanes,
            recommended_concurrency: value.recommended_concurrency,
            reason_code: value.reason_code,
        }
    }
}

impl BrowserOverviewDto {
    fn from_overview(value: BrowserOverview, can_manage_installation: bool) -> Self {
        Self {
            supported: value.supported,
            enabled: value.enabled,
            can_close_all: can_manage_installation,
            can_manage_browser_settings: can_manage_installation,
            can_manage_primary_identity: can_manage_installation,
            running_lanes: value.running_lanes,
            queued_lanes: value.queued_lanes,
            total_lanes: value.total_lanes,
            // The installation overview carries global authority counts;
            // `overview_for_user` computes the same fields from that user's
            // owned Hosts and retained cleanup records.
            managed_host_count: value.managed_host_count,
            pending_cleanup_count: value.pending_cleanup_count,
            pressure_state: value.pressure_state,
            capacity: value.capacity.into(),
            hosts: value
                .hosts
                .into_iter()
                .map(|host| BrowserHostDto {
                    host_id: host.host_id,
                    state: host.state,
                    epoch: host.epoch,
                    headful: host.headful,
                    identity_mode: host.identity_mode,
                    lane_count: host.lane_count,
                    rss_bytes: host.rss_bytes,
                })
                .collect(),
            updated_at: value.updated_at_ms,
        }
    }
}

#[derive(Debug, Serialize)]
struct BrowserLaneDto {
    lane_id: BrowserLaneId,
    lane_name: String,
    lifecycle_state: LaneLifecycleState,
    browser_epoch: u64,
    user_id: String,
    conversation_id: Option<String>,
    runtime_instance_id: String,
    execution_id: Option<String>,
    attempt_id: Option<String>,
    agent_id: Option<String>,
    surface: BrowserSurface,
    owner: BrowserLaneOwnerDto,
    identity: BrowserLaneIdentityDto,
    queue: Option<BrowserLaneQueueDto>,
    tabs: Vec<BrowserTabDto>,
    active_tab_id: Option<String>,
    title: Option<String>,
    url: Option<String>,
    last_active_at: u64,
    created_at: u64,
    resource_estimate_bytes: u64,
    active_operation: bool,
    active_operation_count: usize,
    error_code: Option<BrowserErrorCode>,
    error_message: Option<String>,
    recoverable: bool,
}

/// Renderer-safe tab projection.
///
/// `BrowserTabSnapshot::target_id` is an internal CDP routing detail. The
/// public `tab_id` is the only tab handle exposed to browser-management
/// clients. It is an inventory identifier, not a page-control capability.
#[derive(Debug, Serialize)]
struct BrowserTabDto {
    tab_id: String,
    title: Option<String>,
    url: Option<String>,
    active: bool,
    crashed: bool,
}

impl From<BrowserTabSnapshot> for BrowserTabDto {
    fn from(value: BrowserTabSnapshot) -> Self {
        Self {
            tab_id: value.tab_id,
            title: value.title,
            url: value.url.as_deref().map(project_renderer_url),
            active: value.active,
            crashed: value.crashed,
        }
    }
}

#[derive(Debug, Serialize)]
struct BrowserLaneOwnerDto {
    user_id: String,
    conversation_id: Option<String>,
    runtime_instance_id: String,
    execution_id: Option<String>,
    attempt_id: Option<String>,
    agent_id: Option<String>,
    surface: BrowserSurface,
}

#[derive(Debug, Serialize)]
struct BrowserLaneIdentityDto {
    mode: BrowserIdentityMode,
    generation: u64,
    shared_live: bool,
}

#[derive(Debug, Serialize)]
struct BrowserLaneQueueDto {
    position: usize,
    reason_code: String,
    retry_delay_ms: u64,
    recommended_concurrency: usize,
    owner_active: usize,
    owner_queued: usize,
    global_active: usize,
    global_queued: usize,
}

impl From<QueueMetadata> for BrowserLaneQueueDto {
    fn from(value: QueueMetadata) -> Self {
        Self {
            position: value.position,
            reason_code: value.reason_code,
            retry_delay_ms: value.retry_delay_ms,
            recommended_concurrency: value.recommended_concurrency,
            owner_active: value.owner_active,
            owner_queued: value.owner_queued,
            global_active: value.global_active,
            global_queued: value.global_queued,
        }
    }
}

impl From<BrowserLaneSnapshot> for BrowserLaneDto {
    fn from(value: BrowserLaneSnapshot) -> Self {
        let active_tab = value
            .active_tab_id
            .as_deref()
            .and_then(|active| value.tabs.iter().find(|tab| tab.tab_id == active))
            .or_else(|| value.tabs.iter().find(|tab| tab.active));
        let title = active_tab.and_then(|tab| tab.title.clone());
        let url = active_tab.and_then(|tab| tab.url.clone());
        let caller = value.caller;
        let owner = BrowserLaneOwnerDto {
            user_id: caller.user_id.clone(),
            conversation_id: caller.conversation_id.clone(),
            runtime_instance_id: caller.runtime_instance_id.clone(),
            execution_id: caller.execution_id.clone(),
            attempt_id: caller.attempt_id.clone(),
            agent_id: caller.agent_id.clone(),
            surface: caller.surface,
        };
        Self {
            lane_id: value.lane_id,
            lane_name: value.lane_key.lane_name,
            lifecycle_state: value.lifecycle_state,
            browser_epoch: value.browser_epoch,
            user_id: caller.user_id,
            conversation_id: caller.conversation_id,
            runtime_instance_id: caller.runtime_instance_id,
            execution_id: caller.execution_id,
            attempt_id: caller.attempt_id,
            agent_id: caller.agent_id,
            surface: caller.surface,
            owner,
            identity: BrowserLaneIdentityDto {
                mode: value.identity_mode,
                generation: value.identity_generation,
                shared_live: value.identity_mode == BrowserIdentityMode::Primary,
            },
            queue: value.queue.map(Into::into),
            tabs: value.tabs.into_iter().map(Into::into).collect(),
            active_tab_id: value.active_tab_id,
            title,
            url: url.as_deref().map(project_renderer_url),
            last_active_at: value.last_active_at_ms,
            created_at: value.created_at_ms,
            resource_estimate_bytes: value.resource_estimate_bytes,
            active_operation: value.active_operation_count > 0,
            active_operation_count: value.active_operation_count,
            error_code: value.error_code,
            error_message: value.error_message,
            recoverable: value.recoverable,
        }
    }
}

#[derive(Debug, Serialize)]
struct BrowserScopedCloseResultDto {
    closed: usize,
    already_closed: bool,
    remaining_lane_count: usize,
    remaining_cleanup_count: usize,
    remaining_managed_host_count: usize,
}

#[derive(Debug, Serialize)]
struct BrowserConversationCloseResultDto {
    closed: usize,
    already_closed: bool,
    failed_count: usize,
    failures: Vec<BrowserConversationCloseFailureDto>,
}

#[derive(Debug, Serialize)]
struct BrowserConversationCloseFailureDto {
    lane_id: BrowserLaneId,
    code: BrowserErrorCode,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResourcePolicyPresetDto {
    Automatic,
    ResourceSaving,
    HighConcurrency,
    /// The user's explicit advanced values are authoritative. The scheduler
    /// honors a Custom `reserved_memory_bytes` exactly instead of raising it
    /// to the machine-size floor that protects the derived presets.
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourcePolicyAdvancedDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_memory_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reserved_memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_active_operations: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_open_lanes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_queued_requests: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_owner_queued_requests: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourcePolicyDto {
    preset: ResourcePolicyPresetDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    advanced: Option<ResourcePolicyAdvancedDto>,
}

async fn get_overview(
    State(state): State<BrowserManagementState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<BrowserOverviewDto>>, BrowserApiError> {
    let hub = state.require_hub()?;
    let can_manage_installation = state.is_installation_owner(&user);
    let overview = if can_manage_installation {
        hub.overview().await
    } else {
        hub.overview_for_user(user.id.as_str()).await
    };
    Ok(Json(ApiResponse::ok(BrowserOverviewDto::from_overview(
        overview,
        can_manage_installation,
    ))))
}

async fn get_lanes(
    State(state): State<BrowserManagementState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<BrowserLaneDto>>>, BrowserApiError> {
    let hub = state.require_hub()?;
    let lanes = hub
        .list_lanes()
        .await
        .into_iter()
        .filter(|lane| lane.caller.user_id == user.id.as_str())
        .map(Into::into)
        .collect();
    Ok(Json(ApiResponse::ok(lanes)))
}

async fn close_lane(
    State(state): State<BrowserManagementState>,
    Extension(user): Extension<CurrentUser>,
    Path(lane_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserScopedCloseResultDto>>, BrowserApiError> {
    let hub = state.require_hub()?;
    let lane_id = parse_lane_id(lane_id)?;
    authorize_existing_lane(&hub, user.id.as_str(), &lane_id).await?;
    let result = hub.close_lane(&lane_id).await?;
    Ok(Json(ApiResponse::ok(
        scoped_close_result(
            &hub,
            user.id.as_str(),
            state.is_installation_owner(&user),
            result,
        )
        .await,
    )))
}

#[derive(Debug, Serialize)]
struct BrowserForegroundResultDto {
    foregrounded: bool,
    lane_id: BrowserLaneId,
}

#[derive(Debug, Serialize)]
struct BrowserBackgroundResultDto {
    backgrounded: bool,
    lane_id: BrowserLaneId,
}

async fn foreground_lane(
    State(state): State<BrowserManagementState>,
    Extension(user): Extension<CurrentUser>,
    Path(lane_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserForegroundResultDto>>, BrowserApiError> {
    let hub = state.require_hub()?;
    let lane_id = parse_visibility_lane_id(lane_id)?;
    hub.foreground_lane_for_user(user.id.as_str(), &lane_id)
        .await
        .map_err(|error| foreground_api_error(error, &lane_id))?;
    Ok(Json(ApiResponse::ok(BrowserForegroundResultDto {
        foregrounded: true,
        lane_id,
    })))
}

async fn background_lane(
    State(state): State<BrowserManagementState>,
    Extension(user): Extension<CurrentUser>,
    Path(lane_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserBackgroundResultDto>>, BrowserApiError> {
    let hub = state.require_hub()?;
    let lane_id = parse_visibility_lane_id(lane_id)?;
    hub.set_lane_visibility_for_user(
        user.id.as_str(),
        &lane_id,
        BrowserVisibility::Headless,
    )
    .await
    .map_err(|error| foreground_api_error(error, &lane_id))?;
    Ok(Json(ApiResponse::ok(BrowserBackgroundResultDto {
        backgrounded: true,
        lane_id,
    })))
}

fn foreground_api_error(
    error: BrowserPlatformError,
    lane_id: &BrowserLaneId,
) -> BrowserApiError {
    if matches!(
        error.code,
        BrowserErrorCode::OperationNotAllowed | BrowserErrorCode::LaneNotFound
    ) {
        // The platform deliberately distinguishes a missing Lane from a Lane
        // owned by another user for trusted in-process callers. Collapse both
        // at the HTTP boundary so the authenticated API cannot be used to
        // probe another user's inventory.
        BrowserApiError::not_found()
    } else {
        let mut error = BrowserApiError::from(error);
        // The request path is already known to this caller. Keep the public
        // error scoped to that value even if a driver supplied no Lane id.
        error.body.lane_id = Some(lane_id.clone());
        error
    }
}

async fn close_conversation(
    State(state): State<BrowserManagementState>,
    Extension(user): Extension<CurrentUser>,
    Path(conversation_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserConversationCloseResultDto>>, BrowserApiError> {
    let hub = state.require_hub()?;
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return Err(BrowserApiError::invalid_conversation_id(
            "Conversation id must not be empty.",
        ));
    }
    let lane_ids: Vec<_> = hub
        .list_lanes()
        .await
        .into_iter()
        .filter(|lane| {
            lane.caller.user_id == user.id.as_str()
                && lane.caller.conversation_id.as_deref() == Some(conversation_id)
        })
        .map(|lane| lane.lane_id)
        .collect();
    let result = close_lane_ids_best_effort(&hub, lane_ids).await;
    Ok(Json(ApiResponse::ok(result)))
}

async fn close_all(
    State(state): State<BrowserManagementState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CloseResult>>, BrowserApiError> {
    let hub = state.require_hub()?;
    Ok(Json(ApiResponse::ok(hub.close_all().await?)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrowserDisplayModeValueDto {
    Headless,
    External,
}

impl BrowserDisplayModeValueDto {
    fn from_visibility(visibility: BrowserVisibility) -> Self {
        match visibility {
            BrowserVisibility::Headless => Self::Headless,
            BrowserVisibility::Headful => Self::External,
        }
    }

    fn visibility(self) -> BrowserVisibility {
        match self {
            Self::Headless => BrowserVisibility::Headless,
            Self::External => BrowserVisibility::Headful,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserDisplayModeDto {
    display_mode: BrowserDisplayModeValueDto,
}

async fn get_display_mode(
    State(state): State<BrowserManagementState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<BrowserDisplayModeDto>>, BrowserApiError> {
    // A read must not observe the transient live state between PUT applying a
    // Host visibility change and either committing it or rolling it back after
    // a storage failure.
    let _update_guard = state.display_mode_update_gate.lock().await;
    let hub = state.require_hub()?;
    Ok(Json(ApiResponse::ok(BrowserDisplayModeDto {
        display_mode: BrowserDisplayModeValueDto::from_visibility(
            hub.primary_visibility().await,
        ),
    })))
}

async fn put_display_mode(
    State(state): State<BrowserManagementState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<BrowserDisplayModeDto>, JsonRejection>,
) -> Result<Json<ApiResponse<BrowserDisplayModeDto>>, BrowserApiError> {
    let Json(request) = body.map_err(|_| {
        BrowserApiError::invalid_display_mode(
            "The browser display mode body must contain only display_mode with value headless or external.",
        )
    })?;
    let serialized_mode = serde_json::to_string(&request.display_mode)
        .map_err(|_| BrowserApiError::display_mode_storage())?;
    let _update_guard = state.display_mode_update_gate.lock().await;
    let hub = state.require_hub()?;
    let previous = hub.primary_visibility().await;
    let requested = request.display_mode.visibility();
    let applied = hub.set_primary_visibility(requested).await?;

    if let Err(error) = state
        .preferences
        .upsert_batch(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, serialized_mode.as_str()),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_POLICY_VERSION,
            ),
        ])
        .await
    {
        tracing::warn!(%error, "could not persist browser display mode");
        if let Err(rollback_error) = hub.set_primary_visibility(previous).await {
            tracing::error!(
                %rollback_error,
                "could not roll back live browser display mode after persistence failure"
            );
        }
        return Err(BrowserApiError::display_mode_storage());
    }

    Ok(Json(ApiResponse::ok(BrowserDisplayModeDto {
        display_mode: BrowserDisplayModeValueDto::from_visibility(applied),
    })))
}

async fn get_resource_policy(
    State(state): State<BrowserManagementState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<ResourcePolicyDto>>, BrowserApiError> {
    let hub = state.require_hub()?;
    // Startup restores the persisted policy before the Hub becomes reachable.
    // A safe GET must remain observational: reconciling storage here would
    // turn a CSRF-exempt request into a state-changing operation.
    Ok(Json(ApiResponse::ok(resource_policy_dto(
        &hub.resource_policy().await,
    ))))
}

async fn put_resource_policy(
    State(state): State<BrowserManagementState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<ResourcePolicyDto>, JsonRejection>,
) -> Result<Json<ApiResponse<ResourcePolicyDto>>, BrowserApiError> {
    let Json(request) = body.map_err(|_| {
        BrowserApiError::bad_request("The browser resource policy body is invalid.")
    })?;
    let hub = state.require_hub()?;
    let current = hub.resource_policy().await;
    let policy = apply_resource_policy(current.clone(), &request)?;
    policy_update_result(hub.set_resource_policy(policy.clone()).await)?;
    // Persist the fully materialized policy, not a patch. A restart therefore
    // restores the exact live limits even when the request omitted advanced
    // fields that inherited their current hardware-adaptive values.
    let response = resource_policy_dto(&policy);
    let persisted = serde_json::to_string(&response).map_err(|_| BrowserApiError::storage())?;
    if let Err(error) = state
        .preferences
        .upsert_batch(&[(RESOURCE_POLICY_PREF_KEY, persisted.as_str())])
        .await
    {
        tracing::warn!(%error, "could not persist browser resource policy");
        if let Err(rollback_error) =
            policy_update_result(hub.set_resource_policy(current).await)
        {
            tracing::error!(
                %rollback_error,
                "could not roll back browser resource policy after persistence failure"
            );
        }
        return Err(BrowserApiError::storage());
    }
    Ok(Json(ApiResponse::ok(response)))
}

fn parse_lane_id(value: String) -> Result<BrowserLaneId, BrowserApiError> {
    BrowserLaneId::parse(value).map_err(BrowserApiError::from)
}

fn parse_visibility_lane_id(value: String) -> Result<BrowserLaneId, BrowserApiError> {
    BrowserLaneId::parse(value).map_err(|_| BrowserApiError::invalid_browser_lane_id())
}

async fn authorize_existing_lane(
    hub: &BrowserSessionHub,
    user_id: &str,
    lane_id: &BrowserLaneId,
) -> Result<(), BrowserApiError> {
    if let Some(lane) = hub
        .list_lanes()
        .await
        .into_iter()
        .find(|lane| &lane.lane_id == lane_id)
        && lane.caller.user_id != user_id
    {
        // Do not disclose another user's lane existence.
        return Err(BrowserApiError::not_found());
    }
    Ok(())
}

async fn scoped_close_result(
    hub: &BrowserSessionHub,
    user_id: &str,
    can_manage_installation: bool,
    result: CloseResult,
) -> BrowserScopedCloseResultDto {
    if can_manage_installation {
        return BrowserScopedCloseResultDto {
            closed: result.closed,
            already_closed: result.already_closed,
            remaining_lane_count: result.remaining_lane_count,
            remaining_cleanup_count: result.remaining_cleanup_count,
            remaining_managed_host_count: result.remaining_managed_host_count,
        };
    }

    let overview = hub.overview_for_user(user_id).await;
    BrowserScopedCloseResultDto {
        closed: result.closed,
        already_closed: result.already_closed,
        remaining_lane_count: overview.total_lanes,
        remaining_cleanup_count: overview.pending_cleanup_count,
        remaining_managed_host_count: overview.managed_host_count,
    }
}

async fn close_lane_ids_best_effort(
    hub: &BrowserSessionHub,
    lane_ids: Vec<BrowserLaneId>,
) -> BrowserConversationCloseResultDto {
    let mut closed = 0usize;
    let mut failures = Vec::new();
    for lane_id in lane_ids {
        match hub.close_lane(&lane_id).await {
            Ok(result) => {
                closed = closed.saturating_add(result.closed);
            }
            Err(error) => {
                tracing::warn!(
                    lane_id = %lane_id,
                    code = ?error.code,
                    "could not close browser Lane during conversation cleanup"
                );
                failures.push(BrowserConversationCloseFailureDto {
                    lane_id,
                    code: error.code,
                    message: error.message,
                });
            }
        }
    }
    BrowserConversationCloseResultDto {
        closed,
        already_closed: closed == 0 && failures.is_empty(),
        failed_count: failures.len(),
        failures,
    }
}

fn resource_policy_dto(policy: &ResourcePolicy) -> ResourcePolicyDto {
    ResourcePolicyDto {
        preset: match policy.preset {
            ResourcePolicyPreset::Automatic => ResourcePolicyPresetDto::Automatic,
            ResourcePolicyPreset::ResourceSaving => ResourcePolicyPresetDto::ResourceSaving,
            ResourcePolicyPreset::HighConcurrency => ResourcePolicyPresetDto::HighConcurrency,
            // Round-tripping Custom is what keeps an explicit reserve honored
            // across GET reloads, persistence, and startup restore. Collapsing
            // it to Automatic would silently re-enable the 20%-of-total floor.
            ResourcePolicyPreset::Custom => ResourcePolicyPresetDto::Custom,
        },
        advanced: Some(ResourcePolicyAdvancedDto {
            max_memory_ratio: Some(policy.max_browser_memory_ratio),
            reserved_memory_bytes: Some(policy.reserved_memory_bytes),
            max_active_operations: Some(policy.max_active_operations),
            max_open_lanes: Some(policy.max_open_lanes),
            max_queued_requests: Some(policy.max_global_queue),
            max_owner_queued_requests: Some(policy.max_owner_queue),
        }),
    }
}

/// Restore the persisted management DTO into a complete scheduler policy.
///
/// Startup calls this before constructing `BrowserSessionHub`, so scheduler
/// and operation limits are correct before any caller can open a lane. GET
/// reuses it as a reconciliation path. Invalid or unreadable state never
/// prevents application startup.
pub(crate) async fn restore_persisted_resource_policy(
    repository: &dyn IClientPreferenceRepository,
    base: ResourcePolicy,
) -> ResourcePolicy {
    let rows = match repository.get_by_keys(&[RESOURCE_POLICY_PREF_KEY]).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "could not read persisted browser resource policy");
            return base;
        }
    };
    let Some(row) = rows.into_iter().next() else {
        return base;
    };
    let request = match serde_json::from_str::<ResourcePolicyDto>(&row.value) {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(%error, "ignoring invalid persisted browser resource policy");
            return base;
        }
    };
    match apply_resource_policy(base.clone(), &request) {
        Ok(policy) => policy,
        Err(error) => {
            tracing::warn!(
                reason = %error.body.message,
                "ignoring unsafe persisted browser resource policy"
            );
            base
        }
    }
}

fn apply_resource_policy(
    mut policy: ResourcePolicy,
    request: &ResourcePolicyDto,
) -> Result<ResourcePolicy, BrowserApiError> {
    let requested_preset = match request.preset {
        ResourcePolicyPresetDto::Automatic => ResourcePolicyPreset::Automatic,
        ResourcePolicyPresetDto::ResourceSaving => ResourcePolicyPreset::ResourceSaving,
        ResourcePolicyPresetDto::HighConcurrency => ResourcePolicyPreset::HighConcurrency,
        ResourcePolicyPresetDto::Custom => ResourcePolicyPreset::Custom,
    };
    if policy.preset != requested_preset {
        let current_active = policy.max_active_operations.max(1);
        match requested_preset {
            ResourcePolicyPreset::Automatic => {
                policy.max_browser_memory_ratio = 0.4;
                policy.max_active_operations = match policy.preset {
                    ResourcePolicyPreset::ResourceSaving => current_active.saturating_mul(2).min(64),
                    ResourcePolicyPreset::HighConcurrency => (current_active / 2).max(1),
                    _ => current_active,
                };
                policy.max_open_lanes = policy.max_active_operations.saturating_mul(4).min(128);
            }
            ResourcePolicyPreset::ResourceSaving => {
                policy.max_browser_memory_ratio = 0.3;
                policy.max_active_operations = (current_active / 2).max(1);
                policy.max_open_lanes = policy.max_active_operations.saturating_mul(3).min(96);
            }
            ResourcePolicyPreset::HighConcurrency => {
                policy.max_browser_memory_ratio = 0.5;
                policy.max_active_operations = current_active.saturating_mul(2).min(64);
                policy.max_open_lanes = policy.max_active_operations.saturating_mul(4).min(128);
            }
            // Custom keeps every current value: the user takes over from the
            // preset transition deltas, and the advanced overrides below are
            // the only intended changes.
            ResourcePolicyPreset::Custom => {}
        }
        policy.preset = requested_preset;
    }

    if let Some(advanced) = &request.advanced {
        if let Some(value) = advanced.max_memory_ratio {
            if !value.is_finite()
                || !(MIN_BROWSER_MEMORY_RATIO..=MAX_BROWSER_MEMORY_RATIO).contains(&value)
            {
                return Err(BrowserApiError::bad_request(
                    "max_memory_ratio must be between 0.1 and 0.8.",
                ));
            }
            policy.max_browser_memory_ratio = value;
        }
        if let Some(value) = advanced.reserved_memory_bytes {
            if !(MIN_RESERVED_MEMORY_BYTES..=MAX_RESERVED_MEMORY_BYTES).contains(&value) {
                return Err(BrowserApiError::bad_request(
                    "reserved_memory_bytes is outside the supported range.",
                ));
            }
            policy.reserved_memory_bytes = value;
        }
        if let Some(value) = advanced.max_active_operations {
            if !(1..=MAX_ACTIVE_OPERATIONS).contains(&value) {
                return Err(BrowserApiError::bad_request(format!(
                    "max_active_operations must be between 1 and {MAX_ACTIVE_OPERATIONS}.",
                )));
            }
            policy.max_active_operations = value;
        }
        if let Some(value) = advanced.max_open_lanes {
            if !(1..=MAX_OPEN_LANES).contains(&value) {
                return Err(BrowserApiError::bad_request(format!(
                    "max_open_lanes must be between 1 and {MAX_OPEN_LANES}.",
                )));
            }
            policy.max_open_lanes = value;
        }
        if let Some(value) = advanced.max_queued_requests {
            if !(1..=MAX_GLOBAL_QUEUE).contains(&value) {
                return Err(BrowserApiError::bad_request(format!(
                    "max_queued_requests must be between 1 and {MAX_GLOBAL_QUEUE}.",
                )));
            }
            policy.max_global_queue = value;
        }
        if let Some(value) = advanced.max_owner_queued_requests {
            if !(1..=MAX_OWNER_QUEUE).contains(&value) {
                return Err(BrowserApiError::bad_request(format!(
                    "max_owner_queued_requests must be between 1 and {MAX_OWNER_QUEUE}.",
                )));
            }
            policy.max_owner_queue = value;
        }
    }
    if policy.max_owner_queue > policy.max_global_queue {
        return Err(BrowserApiError::bad_request(
            "max_owner_queued_requests cannot exceed max_queued_requests.",
        ));
    }
    policy.validate().map_err(|error| {
        BrowserApiError::bad_request(format!(
            "The browser resource policy is invalid: {error}."
        ))
    })?;
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    use nomifun_auth::{
        AuthState, CookieConfig, InstanceOwnerState, JwtService, auth_middleware, csrf_middleware,
        require_instance_owner_middleware,
    };
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserHostId, BrowserLaneDriver,
        BrowserOperation, BrowserOperationKind, BrowserOperationResult, BrowserSessionHub,
        BrowserSurface, CallerIdentity, DriverOperationContext, HostLaunchRequest,
        HostLifecycleState, HubConfig, LaneLaunchRequest,
    };
    use nomifun_db::{
        DbError, IClientPreferenceRepository, IUserRepository,
        SqliteClientPreferenceRepository, SqliteUserRepository,
        models::ClientPreference,
    };
    use tower::ServiceExt;

    use super::*;

    #[derive(Clone, Default)]
    struct FakeFactory {
        close_failures: Arc<StdMutex<BTreeSet<BrowserLaneId>>>,
        close_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_failure: Arc<StdMutex<Option<BrowserPlatformError>>>,
    }

    struct FakeHost {
        id: BrowserHostId,
        headful: bool,
        close_failures: Arc<StdMutex<BTreeSet<BrowserLaneId>>>,
        close_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_failure: Arc<StdMutex<Option<BrowserPlatformError>>>,
    }

    struct FakeLane {
        lane_id: BrowserLaneId,
        close_failures: Arc<StdMutex<BTreeSet<BrowserLaneId>>>,
        close_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_failure: Arc<StdMutex<Option<BrowserPlatformError>>>,
    }

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeHost {
                id: request.host_id,
                headful: request.headful,
                close_failures: Arc::clone(&self.close_failures),
                close_attempts: Arc::clone(&self.close_attempts),
                foreground_attempts: Arc::clone(&self.foreground_attempts),
                foreground_failure: Arc::clone(&self.foreground_failure),
            }))
        }
    }

    #[async_trait]
    impl BrowserHostDriver for FakeHost {
        fn host_id(&self) -> BrowserHostId {
            self.id.clone()
        }

        fn epoch(&self) -> u64 {
            1
        }

        fn state(&self) -> HostLifecycleState {
            HostLifecycleState::Running
        }

        fn is_headful(&self) -> bool {
            self.headful
        }

        fn process_id(&self) -> Option<u32> {
            Some(4_242)
        }

        async fn open_lane(
            &self,
            request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeLane {
                lane_id: request.lane_id,
                close_failures: Arc::clone(&self.close_failures),
                close_attempts: Arc::clone(&self.close_attempts),
                foreground_attempts: Arc::clone(&self.foreground_attempts),
                foreground_failure: Arc::clone(&self.foreground_failure),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    #[async_trait]
    impl BrowserLaneDriver for FakeLane {
        async fn execute(
            &self,
            _operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult {
                tabs: vec![BrowserTabSnapshot {
                    tab_id: "tab-safe".to_owned(),
                    target_id: "raw-cdp-target-secret".to_owned(),
                    title: Some("Safe tab".to_owned()),
                    url: Some("https://example.test/".to_owned()),
                    active: true,
                    crashed: false,
                }],
                active_tab_id: Some("tab-safe".to_owned()),
                ..BrowserOperationResult::default()
            })
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            self.close_attempts
                .lock()
                .expect("close attempt list must not be poisoned")
                .push(self.lane_id.clone());
            if self
                .close_failures
                .lock()
                .expect("close failure set must not be poisoned")
                .contains(&self.lane_id)
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "The browser lane could not be closed.",
                    true,
                    "Retry cleanup after the browser host recovers.",
                )
                .with_metadata(json!({
                    "profile_path": "C:\\sensitive-profile",
                    "cdp_endpoint": "ws://127.0.0.1:9222/devtools/browser/private",
                })));
            }
            Ok(())
        }

        async fn bring_to_front(&self) -> Result<(), BrowserPlatformError> {
            self.foreground_attempts
                .lock()
                .expect("foreground attempt list must not be poisoned")
                .push(self.lane_id.clone());
            if let Some(error) = self
                .foreground_failure
                .lock()
                .expect("foreground failure must not be poisoned")
                .clone()
            {
                return Err(error);
            }
            Ok(())
        }
    }

    struct TestPreferenceRepository {
        inner: SqliteClientPreferenceRepository,
        fail_writes: AtomicBool,
        block_writes: AtomicBool,
        write_entered: tokio::sync::Semaphore,
        write_release: tokio::sync::Semaphore,
    }

    impl TestPreferenceRepository {
        fn new(inner: SqliteClientPreferenceRepository) -> Self {
            Self {
                inner,
                fail_writes: AtomicBool::new(false),
                block_writes: AtomicBool::new(false),
                write_entered: tokio::sync::Semaphore::new(0),
                write_release: tokio::sync::Semaphore::new(0),
            }
        }

        fn set_fail_writes(&self, fail: bool) {
            self.fail_writes.store(fail, Ordering::Release);
        }

        fn block_writes(&self) {
            self.block_writes.store(true, Ordering::Release);
        }

        async fn wait_for_blocked_write(&self) {
            self.write_entered
                .acquire()
                .await
                .expect("test preference repository must remain open")
                .forget();
        }

        fn release_blocked_write(&self) {
            self.block_writes.store(false, Ordering::Release);
            self.write_release.add_permits(1);
        }
    }

    #[async_trait]
    impl IClientPreferenceRepository for TestPreferenceRepository {
        async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError> {
            self.inner.get_all().await
        }

        async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError> {
            self.inner.get_by_keys(keys).await
        }

        async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError> {
            if self.block_writes.load(Ordering::Acquire) {
                self.write_entered.add_permits(1);
                self.write_release
                    .acquire()
                    .await
                    .expect("test preference repository must remain open")
                    .forget();
            }
            if self.fail_writes.load(Ordering::Acquire) {
                return Err(DbError::Init(
                    "synthetic browser preference write failure".to_owned(),
                ));
            }
            self.inner.upsert_batch(entries).await
        }

        async fn delete_keys(&self, keys: &[&str]) -> Result<(), DbError> {
            self.inner.delete_keys(keys).await
        }
    }

    struct TestApp {
        router: Router,
        management_state: BrowserManagementState,
        owner_user: CurrentUser,
        token: String,
        secondary_token: String,
        secondary_user_id: String,
        csrf: &'static str,
        lane_id: BrowserLaneId,
        caller: CallerIdentity,
        hub: Arc<BrowserSessionHub>,
        close_failures: Arc<StdMutex<BTreeSet<BrowserLaneId>>>,
        close_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_attempts: Arc<StdMutex<Vec<BrowserLaneId>>>,
        foreground_failure: Arc<StdMutex<Option<BrowserPlatformError>>>,
        preferences: Arc<TestPreferenceRepository>,
    }

    async fn test_app(with_hub: bool) -> TestApp {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let user_repo_concrete = Arc::new(SqliteUserRepository::new(database.pool().clone()));
        let user = user_repo_concrete.get_system_user().await.unwrap().unwrap();
        let owner_user = CurrentUser {
            id: user.user_id.clone(),
            username: user.username.clone(),
        };
        let jwt = Arc::new(JwtService::new("browser-management-test-secret".to_owned()));
        let token = jwt.sign(user.user_id.as_str(), &user.username).unwrap();
        let non_owner = user_repo_concrete
            .create_user("browser-management-non-owner", "unused-password-hash")
            .await
            .unwrap();
        let secondary_token = jwt
            .sign(non_owner.user_id.as_str(), &non_owner.username)
            .unwrap();
        let secondary_user_id = non_owner.user_id.to_string();
        let user_repo: Arc<dyn IUserRepository> = user_repo_concrete;
        let factory = FakeFactory::default();
        let close_failures = Arc::clone(&factory.close_failures);
        let close_attempts = Arc::clone(&factory.close_attempts);
        let foreground_attempts = Arc::clone(&factory.foreground_attempts);
        let foreground_failure = Arc::clone(&factory.foreground_failure);
        let hub = Arc::new(BrowserSessionHub::new(
            Arc::new(factory),
            HubConfig::default(),
        ));
        let lease = hub
            .issue_owner_lease(
                user.user_id.as_str(),
                Some("conversation-safe".to_owned()),
                "runtime-safe",
            )
            .unwrap();
        let caller = CallerIdentity {
            user_id: user.user_id.to_string(),
            conversation_id: Some("conversation-safe".to_owned()),
            runtime_instance_id: "runtime-safe".to_owned(),
            agent_id: Some("agent-safe".to_owned()),
            companion_id: None,
            execution_id: Some("execution-safe".to_owned()),
            step_id: None,
            attempt_id: Some("attempt-safe".to_owned()),
            remote_connection_id: None,
            surface: BrowserSurface::Native,
            owner_lease_id: lease.lease_id,
            capability_expires_at_ms: lease.expires_at_ms,
            allowed_operations: BTreeSet::from([BrowserOperationKind::Manage]),
        };
        // Bind the synthetic management capability before opening the test
        // lane. Production callers enter through `BrowserSessionHub::bind`;
        // keeping the fixture on that same path ensures the owner policy is
        // initialized instead of bypassing the fail-closed lease authority.
        let client = hub.bind(caller.clone()).unwrap();
        let lane_id = hub
            .open_lane(
                client.caller(),
                Some("default"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone();

        let preferences = Arc::new(TestPreferenceRepository::new(
            SqliteClientPreferenceRepository::new(database.pool().clone()),
        ));
        let management_preferences: Arc<dyn IClientPreferenceRepository> = preferences.clone();
        let state = BrowserManagementState::new(
            with_hub.then_some(Arc::clone(&hub)),
            management_preferences,
            Arc::from(user.user_id.as_str()),
        );
        let auth_state = AuthState {
            jwt_service: jwt,
            user_repo,
        };
        let owner_state = InstanceOwnerState::new(Arc::from(user.user_id.as_str()));
        let cookie = Arc::new(CookieConfig {
            secure: false,
            same_site: "Lax",
        });
        let user_router = browser_management_user_routes(state.clone())
            .route_layer(middleware::from_fn_with_state(
                auth_state.clone(),
                auth_middleware,
            ));
        let owner_router = browser_management_owner_routes(state.clone())
            .route_layer(middleware::from_fn_with_state(
                owner_state,
                require_instance_owner_middleware,
            ))
            .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware));
        let router = Router::new()
            .merge(user_router)
            .merge(owner_router)
            .layer(middleware::from_fn_with_state(cookie, csrf_middleware));
        TestApp {
            router,
            management_state: state,
            owner_user,
            token,
            secondary_token,
            secondary_user_id,
            csrf: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            lane_id,
            caller,
            hub,
            close_failures,
            close_attempts,
            foreground_attempts,
            foreground_failure,
            preferences,
        }
    }

    fn authorized_request(
        app: &TestApp,
        method: &str,
        uri: impl AsRef<str>,
        csrf: bool,
    ) -> Request<Body> {
        authenticated_request(app, &app.token, method, uri, csrf)
    }

    fn secondary_request(
        app: &TestApp,
        method: &str,
        uri: impl AsRef<str>,
        csrf: bool,
    ) -> Request<Body> {
        authenticated_request(app, &app.secondary_token, method, uri, csrf)
    }

    fn authenticated_request(
        app: &TestApp,
        token: &str,
        method: &str,
        uri: impl AsRef<str>,
        csrf: bool,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri.as_ref())
            .header("authorization", format!("Bearer {token}"));
        if csrf {
            builder = builder
                .header("x-csrf-token", app.csrf)
                .header("cookie", format!("nomifun-csrf-token={}", app.csrf));
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn open_lane_for_user(
        hub: &BrowserSessionHub,
        user_id: &str,
        conversation_id: &str,
        runtime_instance_id: &str,
        lane_name: &str,
    ) -> BrowserLaneId {
        let lease = hub
            .issue_owner_lease(
                user_id,
                Some(conversation_id.to_owned()),
                runtime_instance_id,
            )
            .unwrap();
        let caller = CallerIdentity {
            user_id: user_id.to_owned(),
            conversation_id: Some(conversation_id.to_owned()),
            runtime_instance_id: runtime_instance_id.to_owned(),
            agent_id: Some(format!("agent-{lane_name}")),
            companion_id: None,
            execution_id: Some(format!("execution-{lane_name}")),
            step_id: None,
            attempt_id: Some(format!("attempt-{lane_name}")),
            remote_connection_id: None,
            surface: BrowserSurface::Native,
            owner_lease_id: lease.lease_id,
            capability_expires_at_ms: lease.expires_at_ms,
            allowed_operations: BTreeSet::from([BrowserOperationKind::Manage]),
        };
        let client = hub.bind(caller).unwrap();
        hub.open_lane(
            client.caller(),
            Some(lane_name),
            BrowserIdentityMode::Primary,
            None,
        )
        .await
        .unwrap()
        .lane()
        .lane_id
        .clone()
    }

    fn authorized_json_request(
        app: &TestApp,
        method: &str,
        uri: &str,
        body: Value,
    ) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", app.token))
            .header("x-csrf-token", app.csrf)
            .header("cookie", format!("nomifun-csrf-token={}", app.csrf))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn browser_api_error_preserves_safe_capacity_queue_and_recovery_metadata() {
        let lane_id = BrowserLaneId::parse("lane-safe").unwrap();
        let error = BrowserApiError::from(
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserCapacityQueued,
                "Capacity is temporarily unavailable.",
                true,
                "Retry after capacity is released.",
            )
            .for_lane(lane_id.clone())
            .with_metadata(json!({
                "capacity": {
                    "active": 2,
                    "queued": 3,
                    "max_active": 4,
                    "max_open_lanes": 8,
                    "recommended_concurrency": 2,
                    "reason_code": "system_memory_pressure",
                    "pressure_state": "pressured",
                },
                "queue": {
                    "position": 3,
                    "retry_delay_ms": 750,
                    "retry_after_ms": 900,
                    "recommended_concurrency": 2,
                    "owner_active": 1,
                    "owner_queued": 2,
                    "global_active": 2,
                    "global_queued": 3,
                    "reason_code": "browser_capacity_queued",
                    "request_state": "queued",
                },
                "recovery": {
                    "circuit_open": true,
                    "failures_in_window": 3,
                    "retry_at_ms": 10_000,
                    "retry_after_ms": 2_000,
                    "old_epoch": 4,
                    "new_epoch": 5,
                    "fresh_observe_required": true,
                    "restart_in_progress": true,
                    "requested_generation": 6,
                    "current_generation": null,
                    "snapshot_available": false,
                    "generation_relation": "older",
                    "refresh_required": true,
                    "cleanup_pending": true,
                    "detached_closed": 1,
                },
            })),
        );

        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        let body = serde_json::to_value(error.body).unwrap();
        assert_eq!(body["code"], "browser_capacity_queued");
        assert_eq!(body["message"], "Capacity is temporarily unavailable.");
        assert_eq!(body["retryable"], true);
        assert_eq!(body["next_action"], "Retry after capacity is released.");
        assert_eq!(body["lane_id"], lane_id.as_str());
        assert_eq!(body["metadata"]["capacity"]["active"], 2);
        assert_eq!(
            body["metadata"]["capacity"]["reason_code"],
            "system_memory_pressure"
        );
        assert_eq!(body["metadata"]["capacity"]["pressure_state"], "pressured");
        assert_eq!(body["metadata"]["queue"]["position"], 3);
        assert_eq!(body["metadata"]["queue"]["retry_delay_ms"], 750);
        assert_eq!(body["metadata"]["queue"]["request_state"], "queued");
        assert_eq!(body["metadata"]["recovery"]["circuit_open"], true);
        assert_eq!(body["metadata"]["recovery"]["current_generation"], Value::Null);
        assert_eq!(
            body["metadata"]["recovery"]["fresh_observe_required"],
            true
        );
        assert_eq!(body["metadata"]["recovery"]["detached_closed"], 1);
    }

    #[test]
    fn browser_api_error_metadata_recursively_drops_sensitive_and_unknown_fields() {
        let raw_target_id = "raw-cdp-target-secret";
        let error = BrowserApiError::from(
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser is recovering.",
                true,
                "Wait for recovery and retry.",
            )
            .with_metadata(json!({
                "reason_code": "browser_resource_pressure",
                "recommended_concurrency": 2,
                "fresh_observe_required": true,
                "cookies": [{"name": "session", "value": "secret-cookie"}],
                "storage": {"local_storage": {"token": "secret-storage"}},
                "cdp_endpoint": "ws://127.0.0.1:9222/devtools/browser/internal",
                "debugging_port": 9222,
                "profile_path": "C:\\Users\\rika0\\Chrome\\User Data\\Profile 1",
                "target_id": raw_target_id,
                "unknown": {
                    "queue": {
                        "position": 99,
                        "cookie": "nested-secret-cookie"
                    }
                },
                "capacity": {
                    "active": 1,
                    "reason_code": "unknown_future_reason",
                    "debugging_port": 9333,
                    "unknown": {"recommended_concurrency": 99}
                },
                "queue": {
                    "position": 2,
                    "reason_code": "global_queue_limit",
                    "request_id": "internal-request-id",
                    "target_id": raw_target_id,
                    "recovery": {
                        "circuit_open": true,
                        "cookies": "deep-secret-cookie",
                        "unknown": {"retry_after_ms": 999}
                    }
                },
                "recovery": {
                    "circuit_open": true,
                    "retry_after_ms": 500,
                    "failure_scope": "host",
                    "profile_path": "C:\\secret-profile",
                    "queue": {
                        "position": 4,
                        "storage": "deep-secret-storage",
                        "future_key": 42
                    }
                }
            })),
        );

        let body = serde_json::to_value(error.body).unwrap();
        assert_eq!(
            body["metadata"],
            json!({
                "reason_code": "browser_resource_pressure",
                "recommended_concurrency": 2,
                "fresh_observe_required": true,
                "capacity": {
                    "active": 1
                },
                "queue": {
                    "position": 2,
                    "reason_code": "global_queue_limit",
                    "recovery": {
                        "circuit_open": true
                    }
                },
                "recovery": {
                    "circuit_open": true,
                    "retry_after_ms": 500,
                    "queue": {
                        "position": 4
                    }
                }
            })
        );

        let encoded = body.to_string();
        for forbidden in [
            "cookies",
            "secret-cookie",
            "storage",
            "secret-storage",
            "cdp_endpoint",
            "devtools",
            "debugging_port",
            "9222",
            "9333",
            "profile_path",
            "User Data",
            "secret-profile",
            "target_id",
            raw_target_id,
            "request_id",
            "internal-request-id",
            "failure_scope",
            "unknown",
            "future_key",
            "unknown_future_reason",
            "99",
            "999",
        ] {
            assert!(!encoded.contains(forbidden), "leaked metadata {forbidden}");
        }
    }

    #[test]
    fn browser_api_error_omits_metadata_when_projection_has_no_safe_fields() {
        let body = serde_json::to_value(
            BrowserApiError::from(
                BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "The managed browser is unavailable.",
                    true,
                    "Retry later.",
                )
                .with_metadata(json!({
                    "cookies": "secret-cookie",
                    "arbitrary_future_key": {"retry_after_ms": 500},
                })),
            )
            .body,
        )
        .unwrap();

        assert!(body.get("metadata").is_none());
        assert_eq!(
            body.as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["code", "message", "next_action", "retryable"])
        );
    }

    #[tokio::test]
    async fn management_routes_require_auth_and_csrf() {
        let app = test_app(true).await;
        let unauthenticated = app
            .router
            .clone()
            .oneshot(
                Request::get("/api/browser/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::FORBIDDEN);

        let secondary_lanes = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "GET",
                "/api/browser/lanes",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(secondary_lanes.status(), StatusCode::OK);

        let without_csrf = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/close", app.lane_id),
                false,
            ))
            .await
            .unwrap();
        assert_eq!(without_csrf.status(), StatusCode::FORBIDDEN);

        let foreground_without_csrf = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/foreground", app.lane_id),
                false,
            ))
            .await
            .unwrap();
        assert_eq!(foreground_without_csrf.status(), StatusCode::FORBIDDEN);
        assert!(
            app.foreground_attempts
                .lock()
                .expect("foreground attempts must not be poisoned")
                .is_empty(),
            "CSRF rejection must happen before foreground dispatch"
        );

        let background_without_csrf = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/background", app.lane_id),
                false,
            ))
            .await
            .unwrap();
        assert_eq!(background_without_csrf.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn foreground_route_dispatches_for_the_authenticated_lane_owner() {
        let app = test_app(true).await;
        let uri = format!("/api/browser/lanes/{}/foreground", app.lane_id);

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(&app, "POST", &uri, true))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["foregrounded"], true);
        assert_eq!(body["data"]["lane_id"], app.lane_id.as_str());
        assert_eq!(
            body["data"]
                .as_object()
                .expect("foreground result must be an object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["foregrounded", "lane_id"])
        );
        assert_eq!(
            app.foreground_attempts
                .lock()
                .expect("foreground attempts must not be poisoned")
                .as_slice(),
            &[app.lane_id.clone()]
        );
    }

    #[tokio::test]
    async fn foreground_route_hides_other_users_lane_and_does_not_dispatch() {
        let app = test_app(true).await;

        let response = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/foreground", app.lane_id),
                true,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        assert_eq!(body["code"], "lane_not_found");
        assert!(body.get("lane_id").is_none());
        assert!(
            app.foreground_attempts
                .lock()
                .expect("foreground attempts must not be poisoned")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn foreground_route_rejects_invalid_lane_ids_without_echoing_input() {
        let app = test_app(true).await;
        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/lanes/%20/foreground",
                true,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["code"], "invalid_lane_id");
        assert!(body.get("lane_id").is_none());
        assert!(!body.to_string().contains("InvalidLaneName"));
        assert!(
            app.foreground_attempts
                .lock()
                .expect("foreground attempts must not be poisoned")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn foreground_route_projects_driver_failures_without_internal_metadata() {
        let app = test_app(true).await;
        *app.foreground_failure
            .lock()
            .expect("foreground failure must not be poisoned") =
            Some(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser operation failed.",
                true,
                "Retry the foreground request.",
            )
            .with_metadata(json!({
                "profile_path": "C:\\private\\profile",
                "cdp_endpoint": "ws://127.0.0.1:9222/devtools/browser/private",
                "debug_token": "secret"
            })));

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/foreground", app.lane_id),
                true,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response_json(response).await;
        assert_eq!(body["code"], "browser_unavailable");
        assert_eq!(body["lane_id"], app.lane_id.as_str());
        assert!(body.get("metadata").is_none());
        let encoded = body.to_string();
        for secret in ["profile_path", "cdp_endpoint", "debug_token", "private"] {
            assert!(!encoded.contains(secret), "leaked internal field {secret}");
        }
    }

    #[tokio::test]
    async fn display_mode_owner_api_applies_live_and_persists_explicit_versioned_choice() {
        let app = test_app(true).await;

        let initial = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/display-mode",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(initial.status(), StatusCode::OK);
        let initial = response_json(initial).await;
        assert_eq!(initial["data"]["display_mode"], "headless");

        let updated = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/display-mode",
                json!({"display_mode": "external"}),
            ))
            .await
            .unwrap();
        let updated_status = updated.status();
        let updated = response_json(updated).await;
        assert_eq!(
            updated_status,
            StatusCode::OK,
            "display mode update failed: {updated}"
        );
        assert_eq!(updated["data"]["display_mode"], "external");
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headful,
            "the API response must represent the live Hub policy"
        );
        assert_eq!(
            app.hub.overview().await.hosts[0].headful,
            true,
            "the running Primary Host must converge before the API reports success"
        );

        let persisted = app
            .preferences
            .get_by_keys(&[
                BROWSER_DISPLAY_MODE_PREF_KEY,
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
            ])
            .await
            .unwrap();
        let persisted: std::collections::BTreeMap<_, _> = persisted
            .into_iter()
            .map(|row| (row.key, row.value))
            .collect();
        assert_eq!(
            persisted
                .get(BROWSER_DISPLAY_MODE_PREF_KEY)
                .map(String::as_str),
            Some("\"external\"")
        );
        assert_eq!(
            persisted
                .get(BROWSER_DISPLAY_MODE_VERSION_PREF_KEY)
                .map(String::as_str),
            Some(BROWSER_DISPLAY_MODE_POLICY_VERSION)
        );
    }

    #[tokio::test]
    async fn display_mode_get_waits_until_a_successful_put_is_persisted() {
        let app = test_app(true).await;
        app.preferences.block_writes();

        let put_router = app.router.clone();
        let put_request = authorized_json_request(
            &app,
            "PUT",
            "/api/browser/display-mode",
            json!({"display_mode": "external"}),
        );
        let put_task =
            tokio::spawn(async move { put_router.oneshot(put_request).await.unwrap() });
        app.preferences.wait_for_blocked_write().await;
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headful,
            "the fixture must pause after live apply and before persistence completes"
        );

        let get_state = app.management_state.clone();
        let get_user = app.owner_user.clone();
        let mut get_future =
            Box::pin(get_display_mode(State(get_state), Extension(get_user)));
        assert!(
            get_future.as_mut().now_or_never().is_none(),
            "GET must wait on the display update gate instead of exposing the uncommitted live state"
        );

        app.preferences.release_blocked_write();
        let put_response = tokio::time::timeout(Duration::from_secs(5), put_task)
            .await
            .expect("persisted PUT must finish after storage is released")
            .expect("PUT task must not panic");
        assert_eq!(put_response.status(), StatusCode::OK);

        let Json(get_response) = tokio::time::timeout(Duration::from_secs(5), get_future)
            .await
            .expect("GET must finish after the committed PUT releases the gate")
            .expect("GET handler must return the committed display mode");
        assert_eq!(
            get_response
                .data
                .expect("GET response must contain display mode")
                .display_mode,
            BrowserDisplayModeValueDto::External
        );
    }

    #[tokio::test]
    async fn display_mode_put_strictly_rejects_unknown_values_and_fields() {
        let app = test_app(true).await;
        for body in [
            json!({"display_mode": "embedded"}),
            json!({"display_mode": "headless", "unexpected": true}),
            json!({"mode": "headless"}),
        ] {
            let response = app
                .router
                .clone()
                .oneshot(authorized_json_request(
                    &app,
                    "PUT",
                    "/api/browser/display-mode",
                    body,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let response = response_json(response).await;
            assert_eq!(response["code"], "invalid_browser_display_mode");
        }
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headless
        );
    }

    #[tokio::test]
    async fn display_mode_storage_failure_rolls_back_the_live_policy() {
        let app = test_app(true).await;
        app.preferences.set_fail_writes(true);

        let response = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/display-mode",
                json!({"display_mode": "external"}),
            ))
            .await
            .unwrap();
        let response_status = response.status();
        let response = response_json(response).await;
        assert_eq!(
            response_status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "display mode persistence failure returned the wrong error: {response}"
        );
        assert_eq!(response["code"], "browser_display_mode_storage_failed");
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headless,
            "a failed write must restore the previously effective live policy"
        );
        assert_eq!(
            app.hub.overview().await.hosts[0].headful,
            false,
            "rollback must restore the actual Primary Host visibility too"
        );
    }

    #[tokio::test]
    async fn display_mode_get_waits_until_a_failed_put_is_rolled_back() {
        let app = test_app(true).await;
        app.preferences.set_fail_writes(true);
        app.preferences.block_writes();

        let put_router = app.router.clone();
        let put_request = authorized_json_request(
            &app,
            "PUT",
            "/api/browser/display-mode",
            json!({"display_mode": "external"}),
        );
        let put_task =
            tokio::spawn(async move { put_router.oneshot(put_request).await.unwrap() });
        app.preferences.wait_for_blocked_write().await;
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headful,
            "the fixture must pause after live apply and before the failed write triggers rollback"
        );

        let get_state = app.management_state.clone();
        let get_user = app.owner_user.clone();
        let mut get_future =
            Box::pin(get_display_mode(State(get_state), Extension(get_user)));
        assert!(
            get_future.as_mut().now_or_never().is_none(),
            "GET must not expose a transient live state that the failed PUT will roll back"
        );

        app.preferences.release_blocked_write();
        let put_response = tokio::time::timeout(Duration::from_secs(5), put_task)
            .await
            .expect("failed PUT must finish after storage is released")
            .expect("PUT task must not panic");
        assert_eq!(put_response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let Json(get_response) = tokio::time::timeout(Duration::from_secs(5), get_future)
            .await
            .expect("GET must finish after rollback releases the gate")
            .expect("GET handler must return the rolled-back display mode");
        assert_eq!(
            get_response
                .data
                .expect("GET response must contain display mode")
                .display_mode,
            BrowserDisplayModeValueDto::Headless
        );
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headless
        );
    }

    #[tokio::test]
    async fn background_route_returns_the_authenticated_primary_lane_to_headless() {
        let app = test_app(true).await;
        app.hub
            .set_primary_visibility(BrowserVisibility::Headful)
            .await
            .unwrap();

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/background", app.lane_id),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(response["data"]["backgrounded"], true);
        assert_eq!(response["data"]["lane_id"], app.lane_id.as_str());
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headful,
            "a one-shot Lane visibility action must not rewrite the global default"
        );
        assert_eq!(
            app.hub.overview().await.hosts[0].headful,
            false,
            "the live Primary Host must actually be replaced by a headless Host"
        );
    }

    #[tokio::test]
    async fn background_route_hides_another_users_lane() {
        let app = test_app(true).await;
        app.hub
            .set_primary_visibility(BrowserVisibility::Headful)
            .await
            .unwrap();

        let response = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/background", app.lane_id),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = response_json(response).await;
        assert!(response.get("lane_id").is_none());
        assert_eq!(
            app.hub.primary_visibility().await,
            BrowserVisibility::Headful,
            "an unauthorized visibility request must not mutate the shared Host"
        );
    }

    #[tokio::test]
    async fn overview_exposes_installation_owner_capabilities_for_current_user() {
        let app = test_app(true).await;
        open_lane_for_user(
            app.hub.as_ref(),
            &app.secondary_user_id,
            "conversation-secondary-overview",
            "runtime-secondary-overview",
            "secondary-overview",
        )
        .await;

        let owner = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/overview",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(owner.status(), StatusCode::OK);
        let owner = response_json(owner).await;
        assert_eq!(owner["data"]["can_close_all"], true);
        assert_eq!(owner["data"]["can_manage_browser_settings"], true);
        assert_eq!(owner["data"]["can_manage_primary_identity"], true);
        assert_eq!(
            owner["data"]["total_lanes"], 2,
            "the installation owner overview must include every managed Lane"
        );

        let non_owner = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "GET",
                "/api/browser/overview",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(non_owner.status(), StatusCode::OK);
        let non_owner = response_json(non_owner).await;
        assert_eq!(non_owner["data"]["can_close_all"], false);
        assert_eq!(non_owner["data"]["can_manage_browser_settings"], false);
        assert_eq!(non_owner["data"]["can_manage_primary_identity"], false);
        assert_eq!(
            non_owner["data"]["total_lanes"], 1,
            "a non-owner overview must remain scoped to that authenticated user"
        );
    }

    #[tokio::test]
    async fn non_owner_overview_counts_own_pending_cleanup_but_hides_foreign_cleanup() {
        let app = test_app(true).await;
        let secondary_failed = open_lane_for_user(
            app.hub.as_ref(),
            &app.secondary_user_id,
            "conversation-secondary-pending",
            "runtime-secondary-pending",
            "secondary-pending",
        )
        .await;
        let _secondary_sibling = open_lane_for_user(
            app.hub.as_ref(),
            &app.secondary_user_id,
            "conversation-secondary-sibling",
            "runtime-secondary-sibling",
            "secondary-sibling",
        )
        .await;
        {
            let mut failures = app
                .close_failures
                .lock()
                .expect("close failure set must not be poisoned");
            failures.insert(app.lane_id.clone());
            failures.insert(secondary_failed.clone());
        }

        app.hub
            .close_lane(&app.lane_id)
            .await
            .expect_err("foreign owner cleanup must remain pending beside live siblings");
        let foreign_only = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "GET",
                "/api/browser/overview",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(foreign_only.status(), StatusCode::OK);
        let foreign_only = response_json(foreign_only).await;
        assert_eq!(
            foreign_only["data"]["pending_cleanup_count"], 0,
            "another user's retained cleanup must not leak through overview_for_user"
        );

        app.hub
            .close_lane(&secondary_failed)
            .await
            .expect_err("the user's failed cleanup must remain pending beside its sibling");
        let own_and_foreign = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "GET",
                "/api/browser/overview",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(own_and_foreign.status(), StatusCode::OK);
        let own_and_foreign = response_json(own_and_foreign).await;
        assert_eq!(
            own_and_foreign["data"]["pending_cleanup_count"], 1,
            "the user must see its own retained cleanup without seeing the foreign one"
        );

        app.close_failures
            .lock()
            .expect("close failure set must not be poisoned")
            .clear();
        app.hub
            .close_all()
            .await
            .expect("test cleanup must drain retained Browser resources");
    }

    #[tokio::test]
    async fn empty_conversation_id_uses_the_conversation_error_contract() {
        let app = test_app(true).await;
        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/conversations/%20/close",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["code"], "invalid_conversation_id");
        assert_eq!(body["retryable"], false);
        assert!(
            body["next_action"]
                .as_str()
                .is_some_and(|message| message.contains("conversation id"))
        );
        assert!(!body.to_string().contains("resource policy"));
    }

    #[tokio::test]
    async fn conversation_close_is_idempotent_and_owner_scoped() {
        let app = test_app(true).await;
        let owner_second = open_lane_for_user(
            app.hub.as_ref(),
            app.caller.user_id.as_str(),
            "conversation-safe",
            "runtime-safe-second",
            "second",
        )
        .await;
        let secondary_lane = open_lane_for_user(
            app.hub.as_ref(),
            &app.secondary_user_id,
            "conversation-safe",
            "runtime-secondary-same-conversation",
            "secondary-same-conversation",
        )
        .await;

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/conversations/conversation-safe/close",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["closed"], 2);
        assert_eq!(body["data"]["already_closed"], false);
        assert_eq!(body["data"]["failed_count"], 0);
        assert_eq!(body["data"]["failures"], json!([]));

        let remaining = app.hub.list_lanes().await;
        assert!(
            remaining
                .iter()
                .all(|lane| lane.lane_id != app.lane_id && lane.lane_id != owner_second)
        );
        assert!(
            remaining
                .iter()
                .any(|lane| lane.lane_id == secondary_lane),
            "conversation cleanup must not close another user's Lane"
        );

        let repeated = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/conversations/conversation-safe/close",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(repeated.status(), StatusCode::OK);
        let repeated = response_json(repeated).await;
        assert_eq!(repeated["data"]["closed"], 0);
        assert_eq!(repeated["data"]["already_closed"], true);
        assert_eq!(repeated["data"]["failed_count"], 0);
        assert_eq!(repeated["data"]["failures"], json!([]));
    }

    #[tokio::test]
    async fn conversation_close_attempts_every_lane_and_reports_safe_failures() {
        let app = test_app(true).await;
        let successful_lane = open_lane_for_user(
            app.hub.as_ref(),
            app.caller.user_id.as_str(),
            "conversation-safe",
            "runtime-safe-after-failure",
            "zz-success-after-failure",
        )
        .await;
        app.close_failures
            .lock()
            .expect("close failure set must not be poisoned")
            .insert(app.lane_id.clone());

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/conversations/conversation-safe/close",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["closed"], 1);
        assert_eq!(body["data"]["already_closed"], false);
        assert_eq!(body["data"]["failed_count"], 1);
        assert_eq!(body["data"]["failures"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            body["data"]["failures"][0]["lane_id"],
            app.lane_id.as_str()
        );
        assert_eq!(
            body["data"]["failures"][0]["code"],
            "browser_unavailable"
        );
        assert_eq!(
            body["data"]["failures"][0]["message"],
            "The browser lane could not be closed."
        );
        let encoded = body.to_string();
        for forbidden in [
            "profile_path",
            "sensitive-profile",
            "cdp_endpoint",
            "devtools",
            "9222",
            "next_action",
            "metadata",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "bulk-close failure leaked {forbidden}"
            );
        }

        let attempts = app
            .close_attempts
            .lock()
            .expect("close attempt list must not be poisoned")
            .clone();
        assert!(attempts.contains(&app.lane_id));
        assert!(
            attempts.contains(&successful_lane),
            "a failed Lane cleanup must not prevent later Lane close attempts"
        );
        assert!(
            app.hub
                .list_lanes()
                .await
                .iter()
                .all(|lane| lane.lane_id != successful_lane),
            "the successful sibling Lane must still be detached"
        );
    }

    #[tokio::test]
    async fn authenticated_users_manage_only_their_own_lanes() {
        let app = test_app(true).await;
        let secondary_lane = open_lane_for_user(
            app.hub.as_ref(),
            &app.secondary_user_id,
            "conversation-secondary",
            "runtime-secondary",
            "secondary",
        )
        .await;

        let listed = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "GET",
                "/api/browser/lanes",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = response_json(listed).await;
        assert_eq!(listed["data"].as_array().map(Vec::len), Some(1));
        assert_eq!(listed["data"][0]["lane_id"], secondary_lane.as_str());
        assert!(!listed.to_string().contains(app.lane_id.as_str()));

        let response = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{}/close", app.lane_id),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "another user's lane must be indistinguishable from a missing lane"
        );

        let close_own = app
            .router
            .clone()
            .oneshot(secondary_request(
                &app,
                "POST",
                format!("/api/browser/lanes/{secondary_lane}/close"),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(close_own.status(), StatusCode::OK);
        let close_own = response_json(close_own).await;
        assert_eq!(close_own["data"]["closed"], 1);
        assert_eq!(close_own["data"]["remaining_lane_count"], 0);
        assert_eq!(close_own["data"]["remaining_managed_host_count"], 0);
        assert_eq!(
            close_own["data"]["remaining_cleanup_count"], 0,
            "an ordinary user receives only its own retained cleanup count"
        );

        assert!(
            app.hub
                .list_lanes()
                .await
                .iter()
                .any(|lane| lane.lane_id == app.lane_id),
            "closing the secondary user's Lane must not touch the owner's Lane"
        );
    }

    #[tokio::test]
    async fn global_browser_controls_remain_installation_owner_only() {
        let app = test_app(true).await;

        for (method, path, csrf) in [
            ("POST", "/api/browser/close-all", true),
            ("GET", "/api/browser/resource-policy", false),
            ("PUT", "/api/browser/resource-policy", true),
            ("GET", "/api/browser/display-mode", false),
            ("PUT", "/api/browser/display-mode", true),
        ] {
            let response = app
                .router
                .clone()
                .oneshot(secondary_request(&app, method, path, csrf))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{method} {path} must remain installation-owner gated"
            );
        }
    }

    #[tokio::test]
    async fn installation_owner_close_all_closes_every_users_lane() {
        let app = test_app(true).await;
        let secondary_lane = open_lane_for_user(
            app.hub.as_ref(),
            &app.secondary_user_id,
            "conversation-secondary-close-all",
            "runtime-secondary-close-all",
            "secondary-close-all",
        )
        .await;

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/close-all",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["closed"], 2);
        assert_eq!(body["data"]["already_closed"], false);
        assert!(
            app.hub.list_lanes().await.is_empty(),
            "installation-wide close-all must remove both the owner and secondary user's lanes"
        );

        let repeated = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "POST",
                "/api/browser/close-all",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(repeated.status(), StatusCode::OK);
        let repeated = response_json(repeated).await;
        assert_eq!(repeated["data"]["closed"], 0);
        assert_eq!(repeated["data"]["already_closed"], true);
        assert!(
            app.hub
                .list_lanes()
                .await
                .iter()
                .all(|lane| lane.lane_id != secondary_lane)
        );
    }

    #[tokio::test]
    async fn lane_response_is_safe_and_close_is_idempotent() {
        let app = test_app(true).await;
        app.hub
            .execute(
                &app.caller,
                &app.lane_id,
                BrowserOperation {
                    kind: BrowserOperationKind::Manage,
                    action: "seed_test_inventory".to_owned(),
                    input: Value::Null,
                    expected_browser_epoch: None,
                    target_id: None,
                    frame_id: None,
                    ref_generation: None,
                    may_modify_identity: false,
                },
            )
            .await
            .expect("test inventory operation must succeed");
        let lanes = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/lanes",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(lanes.status(), StatusCode::OK);
        let lanes = response_json(lanes).await;
        let encoded = lanes.to_string();
        assert!(encoded.contains("runtime-safe"));
        assert!(encoded.contains("conversation-safe"));
        assert!(encoded.contains("tab-safe"));
        assert_eq!(
            lanes["data"][0]["browser_epoch"],
            1,
            "the renderer needs the Lane epoch to associate it with the actual Host"
        );
        assert!(!encoded.contains("raw-cdp-target-secret"));
        let tab = lanes["data"][0]["tabs"][0]
            .as_object()
            .expect("management response must contain the seeded tab");
        assert_eq!(
            tab.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            BTreeSet::from(["active", "crashed", "tab_id", "title", "url"])
        );
        for forbidden in [
            "owner_lease_id",
            "capability_expires_at_ms",
            "allowed_operations",
            "remote_connection_id",
            "active_frame_id",
            "ref_generation",
            "target_id",
            "cdp_endpoint",
            "profile_path",
        ] {
            assert!(!encoded.contains(forbidden), "leaked field {forbidden}");
        }

        let uri = format!("/api/browser/lanes/{}/close", app.lane_id);
        let first = app
            .router
            .clone()
            .oneshot(authorized_request(&app, "POST", &uri, true))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let first = response_json(first).await;
        assert_eq!(first["data"]["closed"], 1);
        assert_eq!(first["data"]["already_closed"], false);

        let second = app
            .router
            .clone()
            .oneshot(authorized_request(&app, "POST", &uri, true))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let second = response_json(second).await;
        assert_eq!(second["data"]["closed"], 0);
        assert_eq!(second["data"]["already_closed"], true);
    }

    #[tokio::test]
    async fn overview_exposes_actual_host_visibility_epoch_and_rss_without_process_id() {
        let app = test_app(true).await;
        app.hub
            .update_resource_telemetry(nomifun_browser_platform::ResourceTelemetry {
                chromium_rss_bytes: 2_048,
                host_rss_by_process_id: std::collections::HashMap::from([(
                    4_242, 2_048,
                )]),
                ..Default::default()
            })
            .await;

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/overview",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["hosts"][0]["rss_bytes"], 2_048);
        assert_eq!(body["data"]["hosts"][0]["epoch"], 1);
        assert_eq!(body["data"]["hosts"][0]["headful"], false);
        let encoded = body.to_string();
        assert!(!encoded.contains("process_id"));
        assert!(!encoded.contains("cdp"));
        assert!(!encoded.contains("profile"));
    }

    #[tokio::test]
    async fn missing_hub_is_explicit_and_safe() {
        let app = test_app(false).await;
        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/overview",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let body = response_json(response).await;
        assert_eq!(body["code"], "browser_not_supported");
        assert!(!body.to_string().contains("profile"));
        assert!(!body.to_string().contains("cdp"));
    }

    #[tokio::test]
    async fn resource_policy_is_validated_persisted_and_applied() {
        let app = test_app(true).await;
        let update = authorized_json_request(
            &app,
            "PUT",
            "/api/browser/resource-policy",
            json!({
                "preset": "resource_saving",
                "advanced": {
                    "max_active_operations": 3,
                    "max_open_lanes": 12,
                    "max_queued_requests": 40,
                    "max_owner_queued_requests": 8
                }
            }),
        );
        let response = app.router.clone().oneshot(update).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["preset"], "resource_saving");
        assert_eq!(body["data"]["advanced"]["max_active_operations"], 3);
        assert_eq!(body["data"]["advanced"]["max_open_lanes"], 12);

        let startup_policy =
            restore_persisted_resource_policy(app.preferences.as_ref(), ResourcePolicy::default())
                .await;
        assert_eq!(startup_policy.preset, ResourcePolicyPreset::ResourceSaving);
        assert_eq!(startup_policy.max_active_operations, 3);
        assert_eq!(startup_policy.max_open_lanes, 12);
        assert_eq!(startup_policy.max_global_queue, 40);
        assert_eq!(startup_policy.max_owner_queue, 8);

        let loaded = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/resource-policy",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(loaded.status(), StatusCode::OK);
        let loaded = response_json(loaded).await;
        assert_eq!(loaded["data"]["preset"], "resource_saving");
        assert_eq!(loaded["data"]["advanced"]["max_queued_requests"], 40);

        let invalid = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/resource-policy",
                json!({
                    "preset": "automatic",
                    "advanced": {"max_active_operations": 65}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        let invalid = response_json(invalid).await;
        assert_eq!(invalid["code"], "invalid_browser_resource_policy");

        let unsafe_persisted =
            r#"{"preset":"automatic","advanced":{"max_active_operations":0}}"#;
        app.preferences
            .upsert_batch(&[(RESOURCE_POLICY_PREF_KEY, unsafe_persisted)])
            .await
            .unwrap();
        let fallback = ResourcePolicy::default();
        let restored =
            restore_persisted_resource_policy(app.preferences.as_ref(), fallback.clone()).await;
        assert_eq!(restored, fallback);
    }

    #[tokio::test]
    async fn custom_preset_preserves_the_exact_explicit_reserve_end_to_end() {
        let app = test_app(true).await;
        // An explicitly configured reserve below the automatic 20%-of-total
        // floor. The scheduler honors it only while the policy carries the
        // Custom preset, so the whole HTTP -> domain -> persistence chain must
        // keep that tag and the exact byte value.
        let explicit_reserve = 2_u64 * 1024 * 1024 * 1024;
        let response = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/resource-policy",
                json!({
                    "preset": "custom",
                    "advanced": {"reserved_memory_bytes": explicit_reserve}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["preset"], "custom");
        assert_eq!(
            body["data"]["advanced"]["reserved_memory_bytes"],
            explicit_reserve
        );

        let live = app.hub.resource_policy().await;
        assert_eq!(live.preset, ResourcePolicyPreset::Custom);
        assert_eq!(live.reserved_memory_bytes, explicit_reserve);

        // A restart restores the same Custom preset and exact reserve, so the
        // scheduler keeps honoring the user's value instead of re-flooring it.
        let restored = restore_persisted_resource_policy(
            app.preferences.as_ref(),
            ResourcePolicy::default(),
        )
        .await;
        assert_eq!(restored.preset, ResourcePolicyPreset::Custom);
        assert_eq!(restored.reserved_memory_bytes, explicit_reserve);
    }

    #[tokio::test]
    async fn resource_policy_get_is_observational_and_does_not_reconcile_storage() {
        let app = test_app(true).await;
        let live = app.hub.resource_policy().await;
        let mut persisted = live.clone();
        persisted.preset = ResourcePolicyPreset::ResourceSaving;
        persisted.max_active_operations = live.max_active_operations.saturating_sub(1).max(1);
        if persisted == live {
            persisted.max_open_lanes = live.max_open_lanes.saturating_sub(1).max(1);
        }
        app.preferences
            .upsert_batch(&[(
                RESOURCE_POLICY_PREF_KEY,
                serde_json::to_string(&resource_policy_dto(&persisted))
                    .unwrap()
                    .as_str(),
            )])
            .await
            .unwrap();

        let response = app
            .router
            .clone()
            .oneshot(authorized_request(
                &app,
                "GET",
                "/api/browser/resource-policy",
                false,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body["data"],
            serde_json::to_value(resource_policy_dto(&live)).unwrap()
        );
        assert_eq!(
            app.hub.resource_policy().await,
            live,
            "a CSRF-exempt GET must never mutate the live scheduler policy"
        );
    }

    #[tokio::test]
    async fn resource_policy_queue_limits_match_platform_caps() {
        let app = test_app(true).await;
        let at_caps = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/resource-policy",
                json!({
                    "preset": "automatic",
                    "advanced": {
                        "max_queued_requests": MAX_GLOBAL_QUEUE,
                        "max_owner_queued_requests": MAX_OWNER_QUEUE
                    }
                }),
            ))
            .await
            .unwrap();
        assert_eq!(at_caps.status(), StatusCode::OK);

        let global_overflow = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/resource-policy",
                json!({
                    "preset": "automatic",
                    "advanced": {"max_queued_requests": MAX_GLOBAL_QUEUE + 1}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(global_overflow.status(), StatusCode::BAD_REQUEST);
        let global_overflow = response_json(global_overflow).await;
        assert!(
            global_overflow["message"]
                .as_str()
                .is_some_and(|message| message.contains(&MAX_GLOBAL_QUEUE.to_string()))
        );

        let owner_overflow = app
            .router
            .clone()
            .oneshot(authorized_json_request(
                &app,
                "PUT",
                "/api/browser/resource-policy",
                json!({
                    "preset": "automatic",
                    "advanced": {"max_owner_queued_requests": MAX_OWNER_QUEUE + 1}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(owner_overflow.status(), StatusCode::BAD_REQUEST);
        let owner_overflow = response_json(owner_overflow).await;
        assert!(
            owner_overflow["message"]
                .as_str()
                .is_some_and(|message| message.contains(&MAX_OWNER_QUEUE.to_string()))
        );
    }

    #[test]
    fn resource_policy_rejects_unsafe_limits() {
        let request = ResourcePolicyDto {
            preset: ResourcePolicyPresetDto::Automatic,
            advanced: Some(ResourcePolicyAdvancedDto {
                max_active_operations: Some(65),
                ..ResourcePolicyAdvancedDto::default()
            }),
        };
        assert!(apply_resource_policy(ResourcePolicy::default(), &request).is_err());
    }

    #[tokio::test]
    async fn lane_dto_projects_renderer_urls_without_mutating_snapshot_urls() {
        let app = test_app(true).await;
        let mut snapshot = app
            .hub
            .list_lanes()
            .await
            .into_iter()
            .find(|lane| lane.lane_id == app.lane_id)
            .expect("test lane must exist");
        let exact_url = "https://alice:password@example.test/callback?safe=yes&Access_Token=secret-token&session-id=secret-session#oauth-fragment".to_owned();
        snapshot.tabs = vec![BrowserTabSnapshot {
            tab_id: "tab-1".to_owned(),
            target_id: "target-1".to_owned(),
            title: Some("Sensitive callback".to_owned()),
            url: Some(exact_url.clone()),
            active: true,
            crashed: false,
        }];
        snapshot.active_tab_id = Some("tab-1".to_owned());

        let dto = serde_json::to_value(BrowserLaneDto::from(snapshot.clone())).unwrap();
        let expected = "https://example.test/callback";
        assert_eq!(dto["url"], expected);
        assert_eq!(dto["tabs"][0]["url"], expected);
        assert_eq!(dto["tabs"][0]["tab_id"], "tab-1");
        assert_eq!(dto["tabs"][0]["title"], "Sensitive callback");
        assert_eq!(dto["tabs"][0]["active"], true);
        assert_eq!(dto["tabs"][0]["crashed"], false);
        assert!(
            dto["tabs"][0].get("target_id").is_none(),
            "renderer tab DTO must not expose raw CDP target ids"
        );
        assert_eq!(
            snapshot.tabs[0].url.as_deref(),
            Some(exact_url.as_str()),
            "the Hub-facing snapshot value must remain exact"
        );
        let encoded = dto.to_string();
        for secret in [
            "alice",
            "password",
            "secret-token",
            "secret-session",
            "oauth-fragment",
            "target-1",
            "target_id",
        ] {
            assert!(!encoded.contains(secret), "DTO leaked {secret}");
        }
    }

    #[tokio::test]
    async fn lane_dto_fails_closed_for_malformed_renderer_urls() {
        let app = test_app(true).await;
        let mut snapshot = app
            .hub
            .list_lanes()
            .await
            .into_iter()
            .find(|lane| lane.lane_id == app.lane_id)
            .expect("test lane must exist");
        snapshot.tabs = vec![BrowserTabSnapshot {
            tab_id: "tab-malformed".to_owned(),
            target_id: "target-malformed".to_owned(),
            title: None,
            url: Some("not a url?access_token=malformed-secret".to_owned()),
            active: true,
            crashed: false,
        }];
        snapshot.active_tab_id = Some("tab-malformed".to_owned());

        let dto = serde_json::to_value(BrowserLaneDto::from(snapshot)).unwrap();
        assert_eq!(dto["url"], "[REDACTED_URL]");
        assert_eq!(dto["tabs"][0]["url"], "[REDACTED_URL]");
        assert!(dto["tabs"][0].get("target_id").is_none());
        assert!(!dto.to_string().contains("malformed-secret"));
        assert!(!dto.to_string().contains("target-malformed"));
    }
}
