use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{BrowserErrorCode, BrowserPlatformError};

/// Maximum UTF-8 byte length for every identifier that can become part of a
/// Browser task, owner-lease, Lane, queue, or cleanup-authority key.
///
/// This is a per-field structural bound, not a global task/concurrency limit.
pub const MAX_BROWSER_IDENTITY_FIELD_BYTES: usize = 128;

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
                if value.trim().is_empty()
                    || value.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
                {
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

/// Exact runtime-scoped lifecycle and cleanup authority.
///
/// This key deliberately includes the trusted runtime instance. Dropping one
/// runtime must never match (and therefore must never close) a sibling runtime
/// which happens to belong to the same user-visible conversation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeCleanupKey(String);

impl RuntimeCleanupKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for RuntimeCleanupKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// User-visible task family used only for bounded resource accounting.
///
/// A conversation may legitimately own several short-lived runtime instances;
/// they share this key so rotating a runtime cannot rotate away its task quota.
/// This key is never cleanup authority: lifecycle teardown remains scoped by
/// [`RuntimeCleanupKey`] or the exact owner lease.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskResourceFamilyKey(String);

impl TaskResourceFamilyKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Builds the accounting key from main-process-verified identity facts.
    /// Model/tool arguments must never be passed to this constructor.
    pub fn from_trusted_parts(
        user_id: &str,
        conversation_id: Option<&str>,
        runtime_instance_id: &str,
        execution_id: Option<&str>,
        remote_connection_id: Option<&str>,
        surface: BrowserSurface,
    ) -> Self {
        let (kind, logical_id) = if let Some(conversation_id) = conversation_id {
            ("conversation", conversation_id)
        } else if surface == BrowserSurface::User {
            // UserBrowserLogin mints a fresh runtime UUID when a login flow is
            // replaced. The logical user surface is stable and must therefore
            // not receive a fresh resource bucket on every replacement.
            ("user_surface", "user")
        } else if surface == BrowserSurface::Remote {
            // A companion can own several independent Remote MCP sessions, so
            // it is not a task boundary. Without a server-pinned connection
            // id, the trusted session/runtime remains the honest fallback.
            // Cross-reconnect grouping requires a future explicitly signed
            // family id which the owner lease can seal; it must not be guessed.
            remote_connection_id.map_or_else(
                || {
                    execution_id
                        .map(|execution_id| ("execution", execution_id))
                        .unwrap_or(("runtime", runtime_instance_id))
                },
                |remote_connection_id| ("remote", remote_connection_id),
            )
        } else if let Some(execution_id) = execution_id {
            ("execution", execution_id)
        } else if let Some(remote_connection_id) = remote_connection_id {
            ("remote", remote_connection_id)
        } else {
            ("runtime", runtime_instance_id)
        };
        Self(format!(
            "{}:{}{}:{}:{}{}",
            user_id.len(),
            user_id,
            kind.len(),
            kind,
            logical_id.len(),
            logical_id
        ))
    }
}

impl fmt::Display for TaskResourceFamilyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
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
    /// Stable lifecycle/cleanup key for one trusted runtime.
    ///
    /// Owner leases may rotate while a runtime stays alive, `attempt_id` is
    /// optional on several ingress paths, and one conversation may contain
    /// concurrent sibling runtimes. Length-prefixing the trusted user id keeps
    /// independently issued runtime ids from sharing a quota accidentally.
    pub fn task_resource_key(&self) -> String {
        self.runtime_cleanup_key().into_string()
    }

    pub fn runtime_cleanup_key(&self) -> RuntimeCleanupKey {
        RuntimeCleanupKey(format!(
            "{}:{}{}",
            self.user_id.len(),
            self.user_id,
            self.runtime_instance_id
        ))
    }

    /// Stable user-visible task family used only for resource quotas.
    pub fn task_resource_family_key(&self) -> TaskResourceFamilyKey {
        TaskResourceFamilyKey::from_trusted_parts(
            &self.user_id,
            self.conversation_id.as_deref(),
            &self.runtime_instance_id,
            self.execution_id.as_deref(),
            self.remote_connection_id.as_deref(),
            self.surface,
        )
    }

    pub fn validate(&self, now_ms: u64) -> Result<(), BrowserPlatformError> {
        if self.user_id.trim().is_empty()
            || self.user_id.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
            || self.runtime_instance_id.trim().is_empty()
            || self.runtime_instance_id.len()
                > MAX_BROWSER_IDENTITY_FIELD_BYTES
            || self.conversation_id.as_ref().is_some_and(|value| {
                value.trim().is_empty()
                    || value.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
            })
            || [
                self.agent_id.as_ref(),
                self.companion_id.as_ref(),
                self.execution_id.as_ref(),
                self.step_id.as_ref(),
                self.attempt_id.as_ref(),
                self.remote_connection_id.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| {
                value.trim().is_empty()
                    || value.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
            })
            || self.owner_lease_id.as_str().trim().is_empty()
            || self.owner_lease_id.as_str().len()
                > MAX_BROWSER_IDENTITY_FIELD_BYTES
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The browser caller identity is incomplete or exceeds its byte limit.",
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
        if runtime_instance_id.trim().is_empty()
            || runtime_instance_id.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The runtime instance is missing or exceeds its byte limit.",
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

/// Requested display policy for the canonical Primary browser Host.
///
/// Crawl/replica Hosts remain headless regardless of this value. Switching a
/// running Primary Host is an explicit lifecycle transition: the Hub replaces
/// the process and rebinds every live Lane under a fresh browser epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserVisibility {
    Headless,
    Headful,
}

impl BrowserVisibility {
    pub const fn is_headful(self) -> bool {
        matches!(self, Self::Headful)
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
    #[serde(default)]
    pub headful: bool,
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
    /// Elastic, machine-wide pressure threshold. This is not a fixed total
    /// Browser Use quota and may change with current hardware telemetry.
    pub global_memory_pressure_threshold_bytes: u64,
    /// Per-task attributed-memory budget. Attribution is estimated on shared
    /// Hosts; the structural task limits below are exact.
    pub max_task_memory_bytes: u64,
    pub max_task_active_operations: usize,
    pub max_task_open_lanes: usize,
    pub max_task_tabs: usize,
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
    #[serde(default)]
    pub managed_host_count: usize,
    #[serde(default)]
    pub pending_cleanup_count: usize,
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
    #[serde(default)]
    pub remaining_lane_count: usize,
    #[serde(default)]
    pub remaining_cleanup_count: usize,
    #[serde(default)]
    pub remaining_managed_host_count: usize,
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

    #[test]
    fn lane_key_enforces_runtime_utf8_byte_limit() {
        let exact = "é".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES / 2);
        assert!(LaneKey::new(exact, None).is_ok());

        let overlong =
            "é".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES / 2 + 1);
        assert_eq!(
            LaneKey::new(overlong, None).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
    }

    #[test]
    fn caller_identity_rejects_overlong_key_fields() {
        let baseline = CallerIdentity {
            user_id: "user".to_owned(),
            conversation_id: Some("conversation".to_owned()),
            runtime_instance_id: "runtime".to_owned(),
            agent_id: None,
            companion_id: None,
            execution_id: None,
            step_id: None,
            attempt_id: None,
            remote_connection_id: None,
            surface: BrowserSurface::Native,
            owner_lease_id: OwnerLeaseId::new(),
            capability_expires_at_ms: 2,
            allowed_operations: BTreeSet::from([
                BrowserOperationKind::Navigate,
            ]),
        };
        assert!(baseline.validate(1).is_ok());
        let overlong = "x".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES + 1);

        let mut caller = baseline.clone();
        caller.user_id = overlong.clone();
        assert_eq!(
            caller.validate(1).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        let mut caller = baseline.clone();
        caller.runtime_instance_id = overlong.clone();
        assert_eq!(
            caller.validate(1).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        let mut caller = baseline.clone();
        caller.conversation_id = Some(overlong.clone());
        assert_eq!(
            caller.validate(1).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        let mut caller = baseline;
        caller.owner_lease_id = OwnerLeaseId(overlong);
        assert_eq!(
            caller.validate(1).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
    }

    #[test]
    fn conversation_family_is_shared_but_runtime_cleanup_is_not() {
        let caller = CallerIdentity {
            user_id: "user".to_owned(),
            conversation_id: Some("conversation".to_owned()),
            runtime_instance_id: "runtime-a".to_owned(),
            agent_id: None,
            companion_id: None,
            execution_id: None,
            step_id: None,
            attempt_id: None,
            remote_connection_id: None,
            surface: BrowserSurface::Acp,
            owner_lease_id: OwnerLeaseId::new(),
            capability_expires_at_ms: 2,
            allowed_operations: BTreeSet::new(),
        };
        let mut sibling = caller.clone();
        sibling.runtime_instance_id = "runtime-b".to_owned();

        assert_eq!(
            caller.task_resource_family_key(),
            sibling.task_resource_family_key()
        );
        assert_ne!(caller.runtime_cleanup_key(), sibling.runtime_cleanup_key());
    }

    #[test]
    fn distinct_conversations_have_distinct_resource_families() {
        let first = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            Some("conversation-a"),
            "runtime",
            None,
            None,
            BrowserSurface::Acp,
        );
        let second = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            Some("conversation-b"),
            "runtime",
            None,
            None,
            BrowserSurface::Acp,
        );
        assert_ne!(first, second);
    }

    #[test]
    fn absent_conversation_uses_stable_semantic_scope_before_runtime() {
        let login_a = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "browser-login-a",
            None,
            None,
            BrowserSurface::User,
        );
        let login_b = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "browser-login-b",
            None,
            None,
            BrowserSurface::User,
        );
        assert_eq!(login_a, login_b, "login runtime rotation must not rotate quota");

        let execution_a = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "runtime-a",
            Some("execution"),
            None,
            BrowserSurface::Native,
        );
        let execution_b = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "runtime-b",
            Some("execution"),
            None,
            BrowserSurface::Native,
        );
        assert_eq!(execution_a, execution_b);

        let remote_a = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "runtime-a",
            None,
            Some("connection"),
            BrowserSurface::Remote,
        );
        let remote_b = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "runtime-b",
            None,
            Some("connection"),
            BrowserSurface::Remote,
        );
        assert_eq!(remote_a, remote_b);

        let fallback_a = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "runtime-a",
            None,
            None,
            BrowserSurface::Acp,
        );
        let fallback_b = TaskResourceFamilyKey::from_trusted_parts(
            "user",
            None,
            "runtime-b",
            None,
            None,
            BrowserSurface::Acp,
        );
        assert_ne!(fallback_a, fallback_b);

        let remote_session_a = CallerIdentity {
            user_id: "user".to_owned(),
            conversation_id: None,
            runtime_instance_id: "remote-session-a".to_owned(),
            agent_id: None,
            companion_id: Some("shared-companion".to_owned()),
            execution_id: None,
            step_id: None,
            attempt_id: None,
            remote_connection_id: None,
            surface: BrowserSurface::Remote,
            owner_lease_id: OwnerLeaseId::new(),
            capability_expires_at_ms: 2,
            allowed_operations: BTreeSet::new(),
        };
        let mut remote_session_b = remote_session_a.clone();
        remote_session_b.runtime_instance_id = "remote-session-b".to_owned();
        assert_ne!(
            remote_session_a.task_resource_family_key(),
            remote_session_b.task_resource_family_key(),
            "a companion may own independent Remote sessions and is not a task boundary"
        );
    }
}
