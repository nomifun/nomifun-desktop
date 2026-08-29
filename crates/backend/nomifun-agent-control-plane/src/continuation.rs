use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, SnapshotCompatibilityAdmissionResult,
};
use nomifun_api_types::{
    AgentBindingValueDto, AgentPresetDraftDto, AgentPresetEditorTestPlanDto,
    AgentSessionContinuationViewDto, EditorDraftStateDto, EditorRevisionActionDto,
    ForkAgentSessionRequestDto, InstallationTokenStateResponseDto, InstallationTokenStatusDto,
    RemoteCredentialContinuationDto, ResolveAgentPresetPreviewResponse,
    RevokeInstallationTokenResponseDto, RotateInstallationTokenResponseDto,
    SaveAgentPresetRevisionRequest, SnapshotCompatibilityViewDto,
};

use crate::ControlPlaneError;
use crate::wire::wire_cast;

pub const AGENT_SESSION_CREATE_PATH: &str = "/api/agent-sessions";

pub fn editor_test_plan(
    draft_state: EditorDraftStateDto,
    preview: ResolveAgentPresetPreviewResponse,
    draft: AgentPresetDraftDto,
    reason: Option<String>,
) -> Result<AgentPresetEditorTestPlanDto, ControlPlaneError> {
    if !preview.can_create_session {
        return Err(ControlPlaneError::canonical(
            "PRESET_REVISION_SAVE_FAILED",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "Preview must resolve successfully before Test creates a Session",
        ));
    }
    let dirty = draft_state == EditorDraftStateDto::Dirty;
    Ok(AgentPresetEditorTestPlanDto {
        draft_state,
        revision_action: if dirty {
            EditorRevisionActionDto::SaveOrdinaryVisibleRevision
        } else {
            EditorRevisionActionDto::ReuseCurrentRevision
        },
        preview: preview.clone(),
        save_request: dirty.then_some(SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(),
            preview_digest: preview.preview_digest.clone(),
            draft,
            reason,
        }),
        session_create_path: AGENT_SESSION_CREATE_PATH.into(),
        uses_real_typed_resources: true,
        uses_full_auto: true,
    })
}

pub fn continuation_view(
    agent_session_id: &AgentSessionId,
    compatibility: &SnapshotCompatibilityAdmissionResult,
    target_agent_binding: Option<&AgentBindingValue>,
    parent_through_seq: u64,
) -> Result<AgentSessionContinuationViewDto, ControlPlaneError> {
    match compatibility {
        SnapshotCompatibilityAdmissionResult::CompatibleExact { .. } => {
            Ok(AgentSessionContinuationViewDto {
                agent_session_id: agent_session_id.as_ref().to_owned(),
                compatibility: wire_cast(compatibility)?,
                history_read_only: false,
                can_continue_same_session: true,
                requires_explicit_fork: false,
                fork_request: None,
            })
        }
        SnapshotCompatibilityAdmissionResult::ExecutorUnavailable { .. } => {
            let target = target_agent_binding.ok_or_else(|| {
                ControlPlaneError::canonical(
                    "SNAPSHOT_EXECUTOR_UNAVAILABLE",
                    axum::http::StatusCode::CONFLICT,
                    "an explicit compatible target AgentBindingValue is required to fork",
                )
            })?;
            Ok(AgentSessionContinuationViewDto {
                agent_session_id: agent_session_id.as_ref().to_owned(),
                compatibility: wire_cast::<_, SnapshotCompatibilityViewDto>(compatibility)?,
                history_read_only: true,
                can_continue_same_session: false,
                requires_explicit_fork: true,
                fork_request: Some(ForkAgentSessionRequestDto {
                    target_agent_binding: wire_cast::<_, AgentBindingValueDto>(target)?,
                    parent_through_seq,
                    title: None,
                }),
            })
        }
    }
}

pub fn remote_credential_continuation() -> RemoteCredentialContinuationDto {
    RemoteCredentialContinuationDto {
        requires_same_owner: true,
        requires_explicit_agent_session_id: true,
        implicit_session_lookup: false,
        auth_error_code: "REMOTE_AUTH_REQUIRED".into(),
        rest_status: 401,
    }
}

pub fn installation_token_state(
    status: InstallationTokenStatusDto,
) -> InstallationTokenStateResponseDto {
    InstallationTokenStateResponseDto {
        configured: status == InstallationTokenStatusDto::Active,
        status,
        continuation: remote_credential_continuation(),
    }
}

pub fn rotated_installation_token(access_token: String) -> RotateInstallationTokenResponseDto {
    RotateInstallationTokenResponseDto {
        access_token,
        status: InstallationTokenStatusDto::Active,
        shown_once: true,
        existing_sessions_unchanged: true,
        continuation: remote_credential_continuation(),
    }
}

pub fn revoked_installation_token() -> RevokeInstallationTokenResponseDto {
    RevokeInstallationTokenResponseDto {
        status: InstallationTokenStatusDto::Revoked,
        existing_sessions_unchanged: true,
        admitted_operations_continue_to_finite_boundary: true,
        continuation: remote_credential_continuation(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d026_presentation_has_no_implicit_session_lookup() {
        let continuation = remote_credential_continuation();
        assert!(continuation.requires_same_owner);
        assert!(continuation.requires_explicit_agent_session_id);
        assert!(!continuation.implicit_session_lookup);
        assert_eq!(continuation.auth_error_code, "REMOTE_AUTH_REQUIRED");
        assert_eq!(continuation.rest_status, 401);
    }
}
