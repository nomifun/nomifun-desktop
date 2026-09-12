//! Narrow typed Session boundary used by Requirements/AutoWork.
//!
//! The Requirement domain owns claims, retries, verdicts, and durable
//! Requirement facts. The host-owned Session implementation owns turn
//! admission, runtime preparation, delivery receipts, and cancellation. This
//! module keeps that ownership explicit without exposing the host
//! implementation or its runtime registry to the Requirement service or runner.

use std::any::{Any, type_name};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_common::{AgentType, AppError};
use nomifun_db::RequirementConversationTurnAuthority;

use crate::autowork_config::{
    AutoWorkConfigSnapshot, AutoWorkSessionConfigCommand,
};

/// One persisted, enabled AutoWork binding that is safe for a runner to
/// resume. The DTO is deliberately independent of Conversation rows and
/// `extra` JSON; the lookup implementation owns the legacy storage mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAutoWorkBinding {
    pub kind: nomifun_api_types::AutoWorkTargetKind,
    pub target_id: String,
    pub display_name: String,
    pub tag: String,
    pub max_requirements: Option<u32>,
    pub config_revision: String,
}

/// Host projection of one persisted Conversation session that has AutoWork
/// enabled. The host remains responsible for reading the compatible
/// `extra.autowork` fields; Requirement receives no Conversation row or JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledAutoWorkSession {
    pub session_id: String,
    pub display_name: String,
    pub tag: String,
    pub max_requirements: Option<u32>,
    pub config_revision: String,
}

/// One malformed persisted binding that was quarantined while a host lookup
/// continued enumerating healthy sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkBindingIssue {
    pub target_id: Option<String>,
    pub code: &'static str,
    pub detail: String,
}

/// Per-item tolerant host lookup result. Storage/query failure still returns
/// `Err`; malformed rows are reported here and do not suppress valid bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduledAutoWorkSessionScan {
    pub sessions: Vec<ScheduledAutoWorkSession>,
    pub quarantined: Vec<AutoWorkBindingIssue>,
}

/// Minimal host-backed lookup for persisted Conversation AutoWork schedules.
///
/// The canonical Session facade implements this projection from the existing
/// `extra.autowork` fields. It exposes no Session object and owns no runtime or
/// turn authority.
#[async_trait]
pub trait AutoWorkScheduledSessionLookup: Send + Sync {
    async fn list_enabled_scheduled_sessions(
        &self,
        owner_id: &str,
    ) -> Result<ScheduledAutoWorkSessionScan, AppError>;
}

/// Canonical lookup contract for boot-resuming persisted AutoWork sessions.
///
/// Implementations may read the compatible persisted representation they own,
/// but callers receive only validated, enabled bindings. No Session object,
/// runtime registry, or mutable Conversation projection crosses this boundary.
#[async_trait]
pub trait AutoWorkBindingLookup: Send + Sync {
    async fn list_enabled_autowork_bindings(
        &self,
        owner_id: &str,
    ) -> Result<Vec<PersistedAutoWorkBinding>, AppError>;
}

/// Instance identity for runtime capabilities issued by one host-owned Session
/// facade.
///
/// A capability issued by one `NomiCoreSessionOwner` instance cannot be
/// consumed by another instance even when both use the same concrete payload
/// type. The host retains this issuer and uses it for issue/consume checks.
#[derive(Clone)]
pub struct AutoWorkRuntimeLeaseIssuer {
    identity: Arc<()>,
}

impl AutoWorkRuntimeLeaseIssuer {
    pub fn new() -> Self {
        Self {
            identity: Arc::new(()),
        }
    }

    pub fn issue<T>(
        &self,
        owner_id: &str,
        session_id: &str,
        payload: T,
        ensure_active: fn(&T) -> Result<(), AppError>,
    ) -> Result<AutoWorkRuntimeBuildLease, AppError>
    where
        T: Any + Send + Sync,
    {
        validate_scope(owner_id, session_id)?;
        Ok(AutoWorkRuntimeBuildLease {
            issuer: self.clone(),
            owner_id: Arc::from(owner_id),
            session_id: Arc::from(session_id),
            host_type: type_name::<T>(),
            payload: Box::new(payload),
            ensure_active: Box::new(move |payload| {
                let payload = payload.downcast_ref::<T>().ok_or_else(|| {
                    AppError::Conflict(
                        "AutoWork runtime-preparation capability changed host type".to_owned(),
                    )
                })?;
                ensure_active(payload)
            }),
        })
    }

    pub fn issue_snapshot(
        &self,
        owner_id: &str,
        session_id: &str,
        revision: impl Into<String>,
    ) -> Result<AutoWorkSessionSnapshotToken, AppError> {
        validate_scope(owner_id, session_id)?;
        let revision = revision.into();
        if revision.trim().is_empty() {
            return Err(AppError::Conflict(
                "AutoWork Session snapshot revision must not be empty".to_owned(),
            ));
        }
        Ok(AutoWorkSessionSnapshotToken {
            issuer: self.clone(),
            owner_id: Arc::from(owner_id),
            session_id: Arc::from(session_id),
            revision: Arc::from(revision),
        })
    }

    pub fn consume<T>(
        &self,
        lease: AutoWorkRuntimeBuildLease,
        owner_id: &str,
        session_id: &str,
    ) -> Result<T, AppError>
    where
        T: Any + Send,
    {
        lease.ensure_issued_by(self, owner_id, session_id)?;
        let expected = type_name::<T>();
        lease
            .payload
            .downcast::<T>()
            .map(|payload| *payload)
            .map_err(|_| {
                AppError::Conflict(format!(
                    "AutoWork runtime-preparation capability belongs to {}, not {expected}",
                    lease.host_type
                ))
            })
    }

    /// Verify a preparation token immediately before durable admission.
    ///
    /// The host must compute `current_revision` from the same canonical Session
    /// projection used by `prepare_autowork_turn`. A mismatch rejects the turn
    /// before the attachment hook or any model/tool effect can start.
    pub fn validate_snapshot(
        &self,
        snapshot: &AutoWorkSessionSnapshotToken,
        owner_id: &str,
        session_id: &str,
        current_revision: &str,
    ) -> Result<(), AppError> {
        snapshot.ensure_issued_by(self, owner_id, session_id)?;
        if snapshot.revision.as_ref() != current_revision {
            return Err(AppError::Conflict(format!(
                "AutoWork Session {session_id} changed after attachment planning"
            )));
        }
        Ok(())
    }

    fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

impl Default for AutoWorkRuntimeLeaseIssuer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AutoWorkRuntimeLeaseIssuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutoWorkRuntimeLeaseIssuer")
            .finish_non_exhaustive()
    }
}

/// Runtime admission capability issued by the host-owned Session.
///
/// The payload remains the canonical host lease. Requirement code can only
/// verify that it is still active and hand it back to the same Session port;
/// it cannot inspect, clone, or mint Session authority.
pub struct AutoWorkRuntimeBuildLease {
    issuer: AutoWorkRuntimeLeaseIssuer,
    owner_id: Arc<str>,
    session_id: Arc<str>,
    host_type: &'static str,
    payload: Box<dyn Any + Send + Sync>,
    ensure_active: Box<LeaseActivityCheck>,
}

type LeaseActivityCheck =
    dyn Fn(&(dyn Any + Send + Sync)) -> Result<(), AppError> + Send + Sync;

impl AutoWorkRuntimeBuildLease {
    pub fn ensure_active(&self) -> Result<(), AppError> {
        (self.ensure_active)(self.payload.as_ref())
    }

    pub fn ensure_scope(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        validate_scope(owner_id, session_id)?;
        if self.owner_id.as_ref() != owner_id || self.session_id.as_ref() != session_id {
            return Err(AppError::Forbidden(
                "AutoWork runtime-preparation capability was bound to a different owner or Session"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn ensure_snapshot(
        &self,
        snapshot: &AutoWorkSessionSnapshotToken,
    ) -> Result<(), AppError> {
        if !self.issuer.same_instance(&snapshot.issuer)
            || self.owner_id != snapshot.owner_id
            || self.session_id != snapshot.session_id
        {
            return Err(AppError::Conflict(
                "AutoWork preparation snapshot does not belong to this runtime lease".to_owned(),
            ));
        }
        Ok(())
    }

    fn ensure_issued_by(
        &self,
        issuer: &AutoWorkRuntimeLeaseIssuer,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        if !self.issuer.same_instance(issuer) {
            return Err(AppError::Conflict(
                "AutoWork runtime-preparation capability belongs to a different Session host"
                    .to_owned(),
            ));
        }
        self.ensure_scope(owner_id, session_id)
    }
}

impl fmt::Debug for AutoWorkRuntimeBuildLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutoWorkRuntimeBuildLease")
            .field("host_type", &self.host_type)
            .field("owner_id", &self.owner_id)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

/// Opaque revision proof for the canonical Session projection used to plan a
/// turn's prompt and attachments.
#[derive(Clone)]
pub struct AutoWorkSessionSnapshotToken {
    issuer: AutoWorkRuntimeLeaseIssuer,
    owner_id: Arc<str>,
    session_id: Arc<str>,
    revision: Arc<str>,
}

impl AutoWorkSessionSnapshotToken {
    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn ensure_scope(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        validate_scope(owner_id, session_id)?;
        if self.owner_id.as_ref() != owner_id || self.session_id.as_ref() != session_id {
            return Err(AppError::Forbidden(
                "AutoWork Session snapshot was bound to a different owner or Session".to_owned(),
            ));
        }
        Ok(())
    }

    fn ensure_issued_by(
        &self,
        issuer: &AutoWorkRuntimeLeaseIssuer,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        if !self.issuer.same_instance(issuer) {
            return Err(AppError::Conflict(
                "AutoWork Session snapshot belongs to a different Session host".to_owned(),
            ));
        }
        self.ensure_scope(owner_id, session_id)
    }
}

impl fmt::Debug for AutoWorkSessionSnapshotToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutoWorkSessionSnapshotToken")
            .field("owner_id", &self.owner_id)
            .field("session_id", &self.session_id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

fn validate_scope(owner_id: &str, session_id: &str) -> Result<(), AppError> {
    if owner_id.trim().is_empty() || session_id.trim().is_empty() {
        return Err(AppError::Forbidden(
            "AutoWork runtime capability requires non-empty owner and Session identities"
                .to_owned(),
        ));
    }
    Ok(())
}

/// Typed durable receipt returned by the host-owned Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkMessageDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

/// Read-only state of the exact public keyed turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoWorkTurnDeliveryState {
    Missing,
    Accepted { message_id: String },
    Completed(AutoWorkMessageDelivery),
}

#[async_trait]
pub trait AutoWorkPreSendHook: Send + Sync {
    async fn prepare(&self) -> Result<(), AppError>;
}

/// Canonical Session facts AutoWork needs before it can render a prompt.
#[derive(Debug, Clone)]
pub struct AutoWorkSessionPreparation {
    pub agent_type: AgentType,
    pub workspace: String,
    pub snapshot: AutoWorkSessionSnapshotToken,
}

/// Requirement-owned message payload. The host translates it into its
/// canonical Session request after it resolves runtime options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkMessage {
    pub content: String,
    pub files: Vec<String>,
    pub inject_skills: Vec<String>,
    pub hidden: bool,
    pub origin: Option<String>,
    pub channel_platform: Option<String>,
}

/// Per-turn overlay that cannot override canonical Session identity, model,
/// delegation policy, workspace, or creation timestamp.
pub struct AutoWorkRuntimeOverlay {
    pub clear_context: bool,
    pub pre_send_hook: Option<Arc<dyn AutoWorkPreSendHook>>,
}

pub struct AutoWorkTurnRequest {
    pub message: AutoWorkMessage,
    pub runtime_overlay: AutoWorkRuntimeOverlay,
    pub session_snapshot: AutoWorkSessionSnapshotToken,
}

/// Result of reconciling an accepted receipt after process interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoWorkReconciliationDisposition {
    LiveExactOwnerWait,
    ReconciledOrTerminalReRead,
    ExternalProofRequiredFailClosed,
    StaleConflict,
}

/// Exact typed command/query surface AutoWork needs from the host-owned Session.
#[async_trait]
pub trait AutoWorkSessionPort: Send + Sync {
    async fn prepare_autowork_turn(
        &self,
        owner_id: &str,
        session_id: &str,
        build_lease: &AutoWorkRuntimeBuildLease,
    ) -> Result<AutoWorkSessionPreparation, AppError>;

    fn begin_runtime_preparation(
        &self,
        conversation_id: &str,
        requester_user_id: &str,
    ) -> Result<AutoWorkRuntimeBuildLease, AppError>;

    fn user_cancelled_since(&self, conversation_id: &str, since_ms: i64) -> bool;

    async fn cancel_active_turn(&self, conversation_id: &str) -> Result<(), AppError>;

    async fn read_config(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<AutoWorkConfigSnapshot, AppError>;

    async fn save_config(
        &self,
        command: AutoWorkSessionConfigCommand,
    ) -> Result<AutoWorkConfigSnapshot, AppError>;

    /// The host must validate both the runtime lease issuer/scope and
    /// `request.session_snapshot` against the latest canonical Session revision
    /// before durable admission. It must retain that preparation fence through
    /// `pre_send_hook.prepare()` and local turn-owner handoff.
    #[allow(clippy::too_many_arguments)]
    async fn send_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: AutoWorkTurnRequest,
        build_lease: AutoWorkRuntimeBuildLease,
        authority: RequirementConversationTurnAuthority,
    ) -> Result<AutoWorkMessageDelivery, AppError>;

    async fn delivery_result(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: &AutoWorkMessage,
        authority: &RequirementConversationTurnAuthority,
    ) -> Result<Option<AutoWorkMessageDelivery>, AppError>;

    async fn public_turn_delivery_state(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<AutoWorkTurnDeliveryState, AppError>;

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<AutoWorkReconciliationDisposition, AppError>;
}

/// Source-compatible name retained for the existing host composition.
///
/// This is only a trait alias. It does not construct an adapter, own runtime
/// state, or create a second Session authority.
pub use AutoWorkSessionPort as AutoWorkConversationPort;

/// Test-only compatibility names for receipt/reconciliation fakes.
///
/// Production code uses the `AutoWork*` names above. Tests that still model
/// the pre-port Conversation vocabulary can use this module without bringing
/// that vocabulary back into the runner or creating another Session owner.
#[cfg(test)]
pub(crate) mod compatibility {
    pub(crate) use super::{
        AutoWorkMessageDelivery as TestMessageDelivery,
        AutoWorkReconciliationDisposition as TestReconciliationDisposition,
        AutoWorkRuntimeBuildLease as TestRuntimeBuildLease,
        AutoWorkTurnDeliveryState as TestTurnDeliveryState,
    };
}

#[cfg(test)]
mod tests {
    use super::AutoWorkRuntimeLeaseIssuer;

    struct HostLease {
        active: bool,
    }

    #[test]
    fn opaque_runtime_lease_is_bound_to_issuer_owner_and_session() {
        let issuer = AutoWorkRuntimeLeaseIssuer::new();
        let other_issuer = AutoWorkRuntimeLeaseIssuer::new();
        let lease = issuer
            .issue("owner-a", "session-a", HostLease { active: true }, |lease| {
                lease
                    .active
                    .then_some(())
                    .ok_or_else(|| nomifun_common::AppError::Conflict("inactive lease".to_owned()))
            })
            .unwrap();
        lease.ensure_active().expect("active host capability");
        assert!(lease.ensure_scope("owner-a", "session-a").is_ok());
        assert!(lease.ensure_scope("owner-b", "session-a").is_err());
        assert!(
            other_issuer
                .consume::<HostLease>(lease, "owner-a", "session-a")
                .is_err(),
            "a different host instance cannot consume the capability"
        );

        let mismatched = issuer
            .issue("owner-a", "session-a", HostLease { active: true }, |_| Ok(()))
            .unwrap();
        assert!(
            issuer
                .consume::<String>(mismatched, "owner-a", "session-a")
                .is_err(),
            "a different host cannot claim the capability"
        );
    }

    #[test]
    fn preparation_snapshot_must_share_lease_issuer_and_scope() {
        let issuer = AutoWorkRuntimeLeaseIssuer::new();
        let lease = issuer
            .issue("owner", "session", HostLease { active: true }, |_| Ok(()))
            .unwrap();
        let snapshot = issuer
            .issue_snapshot("owner", "session", "session-revision-7")
            .unwrap();
        lease
            .ensure_snapshot(&snapshot)
            .expect("same issuer and scope");
        issuer
            .validate_snapshot(&snapshot, "owner", "session", "session-revision-7")
            .expect("same canonical revision");
        assert!(
            issuer
                .validate_snapshot(&snapshot, "owner", "session", "session-revision-8")
                .is_err()
        );

        let other_snapshot = AutoWorkRuntimeLeaseIssuer::new()
            .issue_snapshot("owner", "session", "session-revision-7")
            .unwrap();
        assert!(lease.ensure_snapshot(&other_snapshot).is_err());
    }
}
