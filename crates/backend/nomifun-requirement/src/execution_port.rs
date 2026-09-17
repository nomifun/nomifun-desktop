//! Typed boundaries used by the Requirements queue controller.
//!
//! AutoWork owns only queue selection and Requirement claim policy. Agent work
//! is submitted to the persistent AgentExecution aggregate, which owns the
//! Step, Attempt, AgentSession turn receipt, retry and recovery lifecycle.
//! This module intentionally exposes no Runtime lease, message-delivery
//! receipt, or Conversation turn mutation API. Waiting-for-user remains a
//! live AgentExecution state rather than becoming a second queue receipt.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_common::{AppError, workspace_path_has_edge_whitespace_segment};

use crate::autowork_config::{AutoWorkConfigSnapshot, AutoWorkSessionConfigCommand};

pub use nomifun_agent_execution::{
    AgentExecutionAutomationPort as AutoWorkExecutionPort,
    AutomationExecutionAdmission as AutoWorkExecutionAdmission,
    AutomationExecutionReceipt as AutoWorkExecutionReceipt,
    AutomationExecutionRequest as AutoWorkExecutionRequest,
    AutomationExecutionSource as AutoWorkExecutionSource,
};

pub(crate) fn autowork_execution_operation_id(
    requirement_id: &str,
    claim_generation: i64,
    claim_token: &str,
) -> String {
    let scope = format!(
        "nomifun-autowork-agent-execution-v1\0{requirement_id}\0{claim_generation}\0{claim_token}"
    );
    format!("autowork:{}", nomifun_auth::token_sha256_hex(&scope))
}

/// Exact workspace admitted by the canonical AgentSession owner.
///
/// The textual path is preserved byte-for-byte (apart from rejecting edge
/// whitespace) because AgentExecution performs a second equality check against
/// the frozen Snapshot binding. Requirement staging may use this value only
/// after the owner-scoped port below has returned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenAutoWorkWorkspace {
    root: String,
}

impl FrozenAutoWorkWorkspace {
    pub fn new(root: impl Into<String>) -> Result<Self, AppError> {
        let root = root.into();
        if root.is_empty()
            || root.trim() != root
            || root.contains('\0')
            || !Path::new(&root).is_absolute()
            || workspace_path_has_edge_whitespace_segment(Path::new(&root))
        {
            return Err(AppError::Conflict(
                "AutoWork frozen workspace must be a non-empty absolute path without edge whitespace"
                    .to_owned(),
            ));
        }
        Ok(Self { root })
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.root)
    }

    pub fn as_str(&self) -> &str {
        &self.root
    }

    pub fn into_path_buf(self) -> PathBuf {
        PathBuf::from(self.root)
    }
}

#[derive(Clone)]
pub struct AutoWorkWorkspaceResolution {
    workspace: Option<FrozenAutoWorkWorkspace>,
    _operation_lease: Arc<dyn Send + Sync>,
}

impl AutoWorkWorkspaceResolution {
    pub fn new(workspace: Option<FrozenAutoWorkWorkspace>) -> Self {
        Self {
            workspace,
            _operation_lease: Arc::new(()),
        }
    }

    pub fn with_operation_lease(
        workspace: Option<FrozenAutoWorkWorkspace>,
        operation_lease: Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            workspace,
            _operation_lease: operation_lease,
        }
    }

    pub fn workspace(&self) -> Option<&FrozenAutoWorkWorkspace> {
        self.workspace.as_ref()
    }

    pub(crate) fn operation_lease(&self) -> Arc<dyn Send + Sync> {
        Arc::clone(&self._operation_lease)
    }
}

impl std::fmt::Debug for AutoWorkWorkspaceResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AutoWorkWorkspaceResolution")
            .field("workspace", &self.workspace)
            .field("operation_lease", &"held")
            .finish()
    }
}

/// Owner-scoped resolver for the workspace frozen in one AgentSession binding.
///
/// Implementations must load the exact saved binding/Snapshot and fail closed
/// for a foreign, deleted, ambiguous, or malformed Session. `None` is valid
/// only when the frozen Agent genuinely has no workspace resource.
#[async_trait]
pub trait AutoWorkWorkspacePort: Send + Sync {
    async fn resolve_frozen_workspace(
        &self,
        owner_id: &str,
        agent_session_id: &str,
    ) -> Result<AutoWorkWorkspaceResolution, AppError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAutoWorkBinding {
    pub kind: nomifun_api_types::AutoWorkTargetKind,
    pub target_id: String,
    pub display_name: String,
    pub tag: String,
    pub max_requirements: Option<u32>,
    pub config_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledAutoWorkSession {
    pub session_id: String,
    pub display_name: String,
    pub tag: String,
    pub max_requirements: Option<u32>,
    pub config_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkBindingIssue {
    pub target_id: Option<String>,
    pub code: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduledAutoWorkSessionScan {
    pub sessions: Vec<ScheduledAutoWorkSession>,
    pub quarantined: Vec<AutoWorkBindingIssue>,
}

#[async_trait]
pub trait AutoWorkScheduledSessionLookup: Send + Sync {
    async fn list_enabled_scheduled_sessions(
        &self,
        owner_id: &str,
    ) -> Result<ScheduledAutoWorkSessionScan, AppError>;
}

#[async_trait]
pub trait AutoWorkBindingLookup: Send + Sync {
    async fn list_enabled_autowork_bindings(
        &self,
        owner_id: &str,
    ) -> Result<Vec<PersistedAutoWorkBinding>, AppError>;
}

/// AgentSession-owned persistence for the queue binding itself. This port
/// cannot start a Runtime or deliver a turn.
#[async_trait]
pub trait AutoWorkSessionConfigPort: Send + Sync {
    async fn read_config(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<AutoWorkConfigSnapshot, AppError>;

    async fn save_config(
        &self,
        command: AutoWorkSessionConfigCommand,
    ) -> Result<AutoWorkConfigSnapshot, AppError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_workspace_requires_exact_absolute_host_path() {
        let absolute = std::env::temp_dir().join("autowork-frozen-workspace");
        let workspace = FrozenAutoWorkWorkspace::new(
            absolute.to_string_lossy().into_owned(),
        )
        .unwrap();
        assert_eq!(workspace.as_path(), absolute.as_path());
        for invalid in ["", "relative/workspace", " /tmp/workspace", "/tmp/workspace "] {
            assert!(FrozenAutoWorkWorkspace::new(invalid).is_err(), "accepted {invalid:?}");
        }
    }
}
