//! Queue-controller boundary for durable AgentExecution work.

use std::collections::BTreeSet;
use std::path::Path;

use async_trait::async_trait;
use nomifun_api_types::AgentBindingValueDto;
use nomifun_common::AppError;

/// Resolve the single physical workspace root frozen into an Agent binding.
/// Interactive collaboration and AutoWork both call this authority boundary;
/// neither may infer a workspace from mutable Conversation metadata or model
/// input.
pub fn resolve_frozen_execution_workspace(
    owner_id: &str,
    binding: &AgentBindingValueDto,
) -> Result<Option<String>, AppError> {
    if owner_id.is_empty() || owner_id.trim() != owner_id {
        return Err(AppError::Forbidden(
            "Agent execution workspace authority requires a canonical owner".to_owned(),
        ));
    }
    let mut roots = BTreeSet::new();
    for resource in binding.typed_resource_bindings.iter().filter(|resource| {
        matches!(
            resource.resource_kind.as_str(),
            "workspace" | "process_session"
        )
    }) {
        if resource.owner_id != owner_id {
            return Err(AppError::Forbidden(
                "Agent execution workspace resource belongs to another owner".to_owned(),
            ));
        }
        let root = resource
            .typed_parameters
            .get("workspace_root")
            .ok_or_else(|| {
                AppError::Conflict(
                    "Agent execution workspace resource has no frozen workspace_root".to_owned(),
                )
            })?;
        validate_workspace_root(root)?;
        roots.insert(root.clone());
    }
    if roots.len() > 1 {
        return Err(AppError::Conflict(
            "Agent execution binding contains conflicting workspace roots".to_owned(),
        ));
    }
    Ok(roots.pop_first())
}

/// Backwards-compatible domain name for the AutoWork caller. The authority
/// rule is shared with interactive Agent collaboration above.
pub fn resolve_frozen_automation_workspace(
    owner_id: &str,
    binding: &AgentBindingValueDto,
) -> Result<Option<String>, AppError> {
    resolve_frozen_execution_workspace(owner_id, binding)
}

/// Revalidate a queue-supplied workspace against the same frozen binding used
/// by staging. The exact string is retained; normalization must not broaden a
/// path or conceal a different physical authority.
pub fn admit_frozen_automation_workspace(
    owner_id: &str,
    binding: Option<&AgentBindingValueDto>,
    requested: Option<&str>,
) -> Result<Option<String>, AppError> {
    let bound = binding
        .map(|binding| resolve_frozen_execution_workspace(owner_id, binding))
        .transpose()?
        .flatten();
    if let Some(requested) = requested {
        validate_workspace_root(requested)?;
        if bound.as_deref() != Some(requested) {
            return Err(AppError::Forbidden(
                "AutoWork workspace is outside the frozen Agent resource binding".to_owned(),
            ));
        }
    }
    Ok(bound)
}

fn validate_workspace_root(root: &str) -> Result<(), AppError> {
    let path = Path::new(root);
    if root.is_empty()
        || root.trim() != root
        || root.contains('\0')
        || !path.is_absolute()
        || nomifun_common::workspace_path_has_edge_whitespace_segment(path)
    {
        return Err(AppError::Conflict(
            "AutoWork workspace_root is not a canonical absolute path".to_owned(),
        ));
    }
    Ok(())
}

/// Stable source identity for exactly one Requirement claim generation.
/// `operation_id` is a digest; an opaque queue claim token must never be stored
/// in AgentExecution facts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationExecutionSource {
    pub requirement_id: String,
    pub claim_generation: i64,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationExecutionRequest {
    pub source: AutomationExecutionSource,
    /// Existing AgentSession whose immutable Agent snapshot becomes the lead
    /// participant. No second user-facing Session is created.
    pub lead_session_id: String,
    pub goal: String,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationExecutionAdmission {
    pub execution_id: String,
}

/// Terminal aggregate state returned to queue policy. Paused/WaitingInput
/// remains an active Execution and keeps this future pending while the owning
/// UI handles the durable decision. Attempt retry and AgentSession turn
/// receipts remain private to AgentExecution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationExecutionReceipt {
    Completed {
        execution_id: String,
        summary: Option<String>,
    },
    CompletedWithFailures {
        execution_id: String,
        summary: Option<String>,
    },
    Failed {
        execution_id: String,
        error: Option<String>,
    },
    /// The underlying Agent turn may have committed external effects, but its
    /// operation-scoped canonical receipt never became observable. Queue
    /// policy must park the Requirement for review and must never replay this
    /// generation as an ordinary failure.
    OutcomeUnknown {
        execution_id: String,
        error: Option<String>,
    },
    Cancelled {
        execution_id: String,
        /// True only when the durable aggregate proves that no Attempt was
        /// admitted. Any admitted Attempt may have produced effects before
        /// cancellation, so absence of a terminal receipt is not replay proof.
        replay_safe: bool,
    },
}

impl AutomationExecutionReceipt {
    pub fn execution_id(&self) -> &str {
        match self {
            Self::Completed { execution_id, .. }
            | Self::CompletedWithFailures { execution_id, .. }
            | Self::Failed { execution_id, .. }
            | Self::OutcomeUnknown { execution_id, .. }
            | Self::Cancelled { execution_id, .. } => execution_id,
        }
    }
}

#[async_trait]
pub trait AgentExecutionAutomationPort: Send + Sync {
    /// Validate the exact source, live frozen Session and any existing replay
    /// before the queue owner mutates attachment staging.
    async fn preflight_automation(
        &self,
        owner_id: &str,
        request: &AutomationExecutionRequest,
    ) -> Result<(), AppError>;

    /// Idempotently persist/start the exact source execution after staging has
    /// completed under the Session operation lease.
    async fn admit_automation(
        &self,
        owner_id: &str,
        request: AutomationExecutionRequest,
    ) -> Result<AutomationExecutionAdmission, AppError>;

    /// Wait until an admitted aggregate reaches a terminal state. A durable
    /// WaitingInput state remains live in AgentExecution UI.
    async fn await_automation(
        &self,
        owner_id: &str,
        admission: &AutomationExecutionAdmission,
    ) -> Result<AutomationExecutionReceipt, AppError>;

    /// Cancel the exact source generation and return its canonical terminal
    /// receipt. A missing generation returns `None`; a racing completion
    /// returns that completion instead of being rewritten as cancellation.
    async fn cancel_automation(
        &self,
        owner_id: &str,
        source: &AutomationExecutionSource,
    ) -> Result<Option<AutomationExecutionReceipt>, AppError>;
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_api_types::{
        AgentBindingValueDto, PresetRevisionRefDto, ResolvedSnapshotRefDto,
        TypedResourceBindingDto,
    };

    use super::resolve_frozen_automation_workspace;

    fn binding(resources: Vec<TypedResourceBindingDto>) -> AgentBindingValueDto {
        AgentBindingValueDto {
            preset_revision_ref: PresetRevisionRefDto {
                preset_id: "preset".to_owned(),
                revision: 1,
                revision_digest: "a".repeat(64),
            },
            resolved_snapshot_ref: ResolvedSnapshotRefDto {
                snapshot_id: "snapshot".to_owned(),
                snapshot_digest: "b".repeat(64),
            },
            typed_resource_bindings: resources,
            binding_version: 1,
        }
    }

    fn resource(kind: &str, owner: &str, root: Option<&str>) -> TypedResourceBindingDto {
        TypedResourceBindingDto {
            binding_id: format!("{kind}-binding"),
            resource_kind: kind.to_owned(),
            resource_id: format!("{kind}-resource"),
            owner_id: owner.to_owned(),
            operations: BTreeSet::new(),
            connection_config_ref: None,
            typed_parameters: root
                .map(|root| BTreeMap::from([("workspace_root".to_owned(), root.to_owned())]))
                .unwrap_or_default(),
        }
    }

    #[test]
    fn workspace_and_process_resources_share_one_exact_admission_rule() {
        let root = std::env::temp_dir().join("autowork-shared-workspace");
        let root = root.to_string_lossy().into_owned();
        let exact = binding(vec![
            resource("workspace", "owner", Some(&root)),
            resource("process_session", "owner", Some(&root)),
        ]);
        assert_eq!(
            resolve_frozen_automation_workspace("owner", &exact).unwrap(),
            Some(root.clone())
        );

        let other = std::env::temp_dir()
            .join("autowork-other-workspace")
            .to_string_lossy()
            .into_owned();
        let conflicting = binding(vec![
            resource("workspace", "owner", Some(&root)),
            resource("process_session", "owner", Some(&other)),
        ]);
        assert!(resolve_frozen_automation_workspace("owner", &conflicting).is_err());
        assert!(
            resolve_frozen_automation_workspace(
                "owner",
                &binding(vec![resource(
                    "process_session",
                    "foreign",
                    Some(&root)
                )]),
            )
            .is_err()
        );
        assert!(
            resolve_frozen_automation_workspace(
                "owner",
                &binding(vec![resource("process_session", "owner", None)]),
            )
            .is_err()
        );
    }
}
