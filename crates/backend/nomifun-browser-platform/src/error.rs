use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::BrowserLaneId;

/// Stable machine-readable browser platform errors.  Wire names are part of
/// the Agent/UI contract and must not contain sensitive browser internals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserErrorCode {
    BrowserCapacityQueued,
    SystemMemoryPressure,
    LaneClosedByUser,
    OwnerLeaseExpired,
    StaleBrowserEpoch,
    StaleLaneRef,
    TargetCrashed,
    BrowserRestarted,
    IdentityReplicaStale,
    NeedsPrimaryIdentity,
    PrimaryProfileStorageLimit,
    LaneNotFound,
    OperationNotAllowed,
    BrowserUnavailable,
    BrowserShuttingDown,
    InvalidCallerIdentity,
    InvalidLaneName,
}

#[derive(Clone, Debug, Error, Serialize, Deserialize)]
#[error("{code:?}: {message}")]
pub struct BrowserPlatformError {
    pub code: BrowserErrorCode,
    /// Safe for direct user display.
    pub message: String,
    pub retryable: bool,
    pub next_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane_id: Option<BrowserLaneId>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl BrowserPlatformError {
    pub fn new(
        code: BrowserErrorCode,
        message: impl Into<String>,
        retryable: bool,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            next_action: next_action.into(),
            lane_id: None,
            metadata: Value::Null,
        }
    }

    pub fn for_lane(mut self, lane_id: BrowserLaneId) -> Self {
        self.lane_id = Some(lane_id);
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn lane_not_found(lane_id: BrowserLaneId) -> Self {
        Self::new(
            BrowserErrorCode::LaneNotFound,
            "The browser lane no longer exists.",
            false,
            "Refresh the browser inventory or open a new lane.",
        )
        .for_lane(lane_id)
    }

    pub fn shutting_down() -> Self {
        Self::new(
            BrowserErrorCode::BrowserShuttingDown,
            "The browser platform is shutting down.",
            true,
            "Retry after the application is ready.",
        )
    }
}
