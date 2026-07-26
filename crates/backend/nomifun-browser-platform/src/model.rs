use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{BrowserErrorCode, BrowserPlatformError};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7().to_string())
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, BrowserPlatformError> {
                let value = value.into();
                if value.trim().is_empty() || value.len() > 128 {
                    return Err(BrowserPlatformError::new(
                        BrowserErrorCode::InvalidCallerIdentity,
                        concat!(stringify!($name), " is invalid."),
                        false,
                        "Request a fresh browser capability.",
                    ));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(BrowserLaneId);
string_id!(BrowserHostId);
string_id!(OwnerLeaseId);
string_id!(QueueRequestId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSurface {
    Native,
    Gateway,
    Acp,
    Remote,
    Cluster,
    User,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserOperationKind {
    Navigate,
    Observe,
    Act,
    Screenshot,
    Tabs,
    Download,
    Debug,
    Manage,
    View,
    Crawl,
}

/// Identity facts signed or resolved by the main process.  Model-provided
/// input may only select `lane_name`; it must never populate these fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerIdentity {
    pub user_id: String,
    pub conversation_id: Option<String>,
    pub runtime_instance_id: String,
    pub agent_id: Option<String>,
    pub companion_id: Option<String>,
    pub execution_id: Option<String>,
    pub step_id: Option<String>,
    pub attempt_id: Option<String>,
    pub remote_connection_id: Option<String>,
    pub surface: BrowserSurface,
    pub owner_lease_id: OwnerLeaseId,
    pub capability_expires_at_ms: u64,
    pub allowed_operations: BTreeSet<BrowserOperationKind>,
}

impl CallerIdentity {
    pub fn validate(&self, now_ms: u64) -> Result<(), BrowserPlatformError> {
        if self.user_id.trim().is_empty()
            || self.runtime_instance_id.trim().is_empty()
            || self.owner_lease_id.as_str().trim().is_empty()
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The browser caller identity is incomplete.",
                false,
                "Request a fresh browser capability from the application.",
            ));
        }
        if self.capability_expires_at_ms <= now_ms {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OwnerLeaseExpired,
                "The browser capability has expired.",
                false,
                "Request a fresh browser capability.",
            ));
        }
        Ok(())
    }

    pub fn allows(&self, operation: BrowserOperationKind) -> bool {
        self.allowed_operations.contains(&operation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LaneKey {
    pub runtime_instance_id: String,
    pub lane_name: String,
}

impl LaneKey {
    pub fn new(
        runtime_instance_id: impl Into<String>,
        lane_name: Option<&str>,
    ) -> Result<Self, BrowserPlatformError> {
        let runtime_instance_id = runtime_instance_id.into();
        if runtime_instance_id.trim().is_empty() {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The runtime instance is missing.",
                false,
                "Request a fresh browser capability.",
            ));
        }
        Ok(Self {
            runtime_instance_id,
            lane_name: normalize_lane_name(lane_name.unwrap_or("default"))?,
        })
    }
}

pub fn normalize_lane_name(value: &str) -> Result<String, BrowserPlatformError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidLaneName,
            "Lane names must be 1-32 letters, numbers, '-' or '_'.",
            false,
            "Choose a short lane name such as 'default' or 'research-2'.",
        ));
    }
    Ok(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserIdentityMode {
    Primary,
    Anonymous,
    AuthenticatedReplica,
    Isolated,
}

impl BrowserIdentityMode {
    /// Only live interactive identity domains may accept user or Agent input.
    /// Explicit enumeration keeps future identity modes fail-closed.
    pub const fn permits_interaction(self) -> bool {
        matches!(self, Self::Primary | Self::Isolated)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneLifecycleState {
    Queued,
    Starting,
    Running,
    Frozen,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneControlState {
    Agent,
    User,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostLifecycleState {
    Stopped,
    Starting,
    Running,
    Restarting,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePressureState {
    Normal,
    Pressured,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerState {
    Idle,
    Starting,
    Streaming,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTabSnapshot {
    pub tab_id: String,
    pub target_id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub active: bool,
    pub crashed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMetadata {
    pub request_id: QueueRequestId,
    pub position: usize,
    pub recommended_concurrency: usize,
    pub owner_active: usize,
    pub owner_queued: usize,
    pub global_active: usize,
    pub global_queued: usize,
    pub retry_delay_ms: u64,
    pub reason_code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserLaneSnapshot {
    pub lane_id: BrowserLaneId,
    pub lane_key: LaneKey,
    pub caller: CallerIdentity,
    pub identity_mode: BrowserIdentityMode,
    pub identity_generation: u64,
    pub lifecycle_state: LaneLifecycleState,
    pub control_state: LaneControlState,
    pub browser_epoch: u64,
    pub tabs: Vec<BrowserTabSnapshot>,
    pub active_tab_id: Option<String>,
    pub active_frame_id: Option<String>,
    pub ref_generation: u64,
    pub queue: Option<QueueMetadata>,
    pub resource_estimate_bytes: u64,
    pub active_operation_count: usize,
    pub last_active_at_ms: u64,
    pub created_at_ms: u64,
    pub viewer_state: ViewerState,
    pub error_code: Option<BrowserErrorCode>,
    pub error_message: Option<String>,
    pub recoverable: bool,
}

impl BrowserLaneSnapshot {
    pub fn conversation_id(&self) -> Option<&str> {
        self.caller.conversation_id.as_deref()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserHostSnapshot {
    pub host_id: BrowserHostId,
    pub state: HostLifecycleState,
    pub epoch: u64,
    pub identity_mode: BrowserIdentityMode,
    pub lane_count: usize,
    pub rss_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserCapacitySnapshot {
    pub active: usize,
    pub queued: usize,
    pub max_active: usize,
    pub max_open_lanes: usize,
    pub recommended_concurrency: usize,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserOverview {
    pub supported: bool,
    pub enabled: bool,
    pub running_lanes: usize,
    pub queued_lanes: usize,
    pub total_lanes: usize,
    pub pressure_state: ResourcePressureState,
    pub capacity: BrowserCapacitySnapshot,
    pub hosts: Vec<BrowserHostSnapshot>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationContext {
    pub browser_epoch: u64,
    pub lane_id: BrowserLaneId,
    pub target_id: Option<String>,
    pub frame_id: Option<String>,
    pub ref_generation: u64,
    pub cancellation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserOperation {
    pub kind: BrowserOperationKind,
    pub action: String,
    #[serde(default)]
    pub input: Value,
    /// Compare-only epoch carried with a previously returned Lane/ref handle.
    /// It grants no authority; the Hub rejects a mismatch before dispatch.
    #[serde(default)]
    pub expected_browser_epoch: Option<u64>,
    pub target_id: Option<String>,
    pub frame_id: Option<String>,
    pub ref_generation: Option<u64>,
    #[serde(default)]
    pub may_modify_identity: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrowserOperationResult {
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub tabs: Vec<BrowserTabSnapshot>,
    pub active_tab_id: Option<String>,
    pub active_frame_id: Option<String>,
    pub ref_generation: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserInventoryEvent {
    pub sequence: u64,
    pub change_kind: String,
    pub lane_id: Option<BrowserLaneId>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CloseResult {
    pub closed: usize,
    pub already_closed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_name_is_normalized_and_bounded() {
        assert_eq!(normalize_lane_name(" Research-2 ").unwrap(), "research-2");
        assert!(normalize_lane_name("../escape").is_err());
        assert!(normalize_lane_name("").is_err());
        assert!(normalize_lane_name(&"a".repeat(33)).is_err());
    }

    #[test]
    fn lane_key_uses_runtime_not_conversation_or_companion() {
        let a = LaneKey::new("runtime-a", Some("default")).unwrap();
        let b = LaneKey::new("runtime-b", Some("default")).unwrap();
        assert_ne!(a, b);
    }
}
