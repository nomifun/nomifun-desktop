use serde::{Deserialize, Serialize};
pub use nomifun_agent_contracts::{ModelFailureDiagnostic, ModelFailureReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorOwnership {
    Nomifun,
    UserAgent,
    UserLlmProvider,
    UnknownUpstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentErrorCode {
    NomifunConversationBusy,
    NomifunStreamBroken,
    NomifunStateInconsistent,
    /// Current capability/Skill provenance differs from the frozen Session.
    /// Admission refuses execution; repeating the same request cannot fix it.
    NomifunSessionConfigurationChanged,
    /// Nomi restored the accepted turn's in-memory root after rejecting an
    /// unsupported completion claim, but could not durably persist that root.
    /// The exact persisted Nomi session must be quarantined and reset before a
    /// replacement runtime may be admitted.
    NomifunAgentSessionInconsistent,
    NomifunPermissionError,
    NomifunInternalError,
    /// The runtime safely stopped after an execution or completion guard was
    /// exhausted. Earlier tool effects may have succeeded and must be checked
    /// before the user continues the task.
    NomifunTaskIncomplete,
    /// Nomi serialized a tool result into a provider function-response shape
    /// whose reserved fields changed the result's meaning. Replaying the same
    /// accepted turn would deterministically reproduce the rejected payload.
    NomifunToolResultEncodingError,
    WorkspacePathEdgeWhitespaceRuntimeUnsupported,
    WorkspaceDirectoryRuntimeUnavailable,
    UserAgentHandshakeFailed,
    UserAgentHandshakeTimeout,
    UserAgentAcpInitFailed,
    UserAgentProtocolMismatch,
    UserAgentNotInstalled,
    UserAgentStartupFailed,
    UserAgentDisconnected,
    UserAgentAuthRequired,
    UserAgentSessionNotFound,
    UserAgentNoPreviousSession,
    UserAgentCommandNotFound,
    UserAgentMissingEnv,
    UserAgentUnsupportedMethod,
    UserAgentInvalidParams,
    UserLlmProviderAuthFailed,
    UserLlmProviderPermissionDenied,
    UserLlmProviderBillingRequired,
    UserLlmProviderConfigError,
    UserLlmProviderModelNotFound,
    UserLlmProviderUnsupportedModel,
    UserLlmProviderEndpointNotFound,
    UserLlmProviderInvalidRequest,
    /// 模型不支持图片输入(收到 image_url 类 400)。会话服务据此剔图重跑,
    /// 故意 **不** 计入 is_provider_fault(不触发换模型)。
    UserLlmProviderImageUnsupported,
    UserLlmProviderInvalidToolSchema,
    UserLlmProviderInvalidToolCall,
    UserLlmProviderContextTooLarge,
    UserLlmProviderRateLimited,
    UserLlmProviderTimeout,
    UserLlmProviderNetworkError,
    UserLlmProviderEmptyResponse,
    /// The provider/model ended normally at the protocol layer but asserted
    /// completion without machine evidence for a required deliverable. This is
    /// a provider-quality failure, not a transient transport fault: changing
    /// model may help, while replaying the same turn automatically is unsafe.
    UserLlmProviderUnbackedCompletion,
    UserLlmProviderGatewayError,
    UserLlmProviderUnavailable,
    UserLlmProviderUnsupportedFeature,
    UserLlmProviderInvalidResponse,
    UserLlmProviderStreamInterrupted,
    UnknownUpstreamError,
}

/// The engine's reason for stopping an unfinished task. This supplements the
/// error code for presentation without changing retry or recovery authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskIncompleteReason {
    BlockedWork,
    StepLimit,
    OutputTruncated,
    UnresolvedPlan,
    UnverifiedChanges,
    RunningProcesses,
    UnverifiedCompletion,
    RejectedControl,
    NoProgress,
    ExecutionGuard,
    RecoveryGuard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorResolutionKind {
    Retry,
    WaitForCurrentResponse,
    StartNewSession,
    ReconnectAgent,
    CheckAgentLogin,
    CheckAgentInstallation,
    CheckAgentVersion,
    CheckLocalCommand,
    CheckProviderCredentials,
    CheckProviderBilling,
    CheckProviderBaseUrl,
    ChangeModel,
    ReduceContext,
    SendFeedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorResolutionTarget {
    ProviderSettings,
    AgentSettings,
    NewConversation,
    Feedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentErrorResolution {
    pub kind: AgentErrorResolutionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<AgentErrorResolutionTarget>,
}

impl AgentErrorResolution {
    pub fn new(kind: AgentErrorResolutionKind, target: Option<AgentErrorResolutionTarget>) -> Self {
        Self { kind, target }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStreamErrorData {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<AgentErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<AgentErrorOwnership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(
        default,
        rename = "workspacePath",
        alias = "workspace_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_path: Option<String>,
    /// Host-captured presentation context for this exact admitted Turn.
    #[serde(default, rename = "agentLabel", alias = "agent_label", skip_serializing_if = "Option::is_none")]
    pub agent_label: Option<String>,
    #[serde(default, rename = "agentTemplateKey", alias = "agent_template_key", skip_serializing_if = "Option::is_none")]
    pub agent_template_key: Option<String>,
    #[serde(default, rename = "modelName", alias = "model_name", skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(
        default,
        rename = "taskIncompleteReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_incomplete_reason: Option<AgentTaskIncompleteReason>,
    /// The original typed model/transport diagnosis, shared with the Broker.
    #[serde(default, rename = "providerDiagnostic", skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic: Option<ModelFailureDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_recommended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<AgentErrorResolution>,
}

impl AgentStreamErrorData {
    pub fn legacy(message: impl Into<String>, code: Option<AgentErrorCode>) -> Self {
        Self {
            message: message.into(),
            code,
            ownership: None,
            detail: None,
            workspace_path: None,
            agent_label: None,
            agent_template_key: None,
            model_name: None,
            task_incomplete_reason: None,
            provider_diagnostic: None,
            retryable: None,
            feedback_recommended: None,
            resolution: None,
        }
    }

    pub fn classified(
        message: impl Into<String>,
        code: AgentErrorCode,
        ownership: AgentErrorOwnership,
        detail: Option<String>,
        retryable: bool,
        feedback_recommended: bool,
        resolution: Option<AgentErrorResolution>,
    ) -> Self {
        Self {
            message: message.into(),
            code: Some(code),
            ownership: Some(ownership),
            detail,
            workspace_path: None,
            agent_label: None,
            agent_template_key: None,
            model_name: None,
            task_incomplete_reason: None,
            provider_diagnostic: None,
            retryable: Some(retryable),
            feedback_recommended: Some(feedback_recommended),
            resolution,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_turn_context_has_public_wire_keys_and_remains_optional() {
        let mut error = AgentStreamErrorData::legacy("failure", None);
        let missing = serde_json::to_value(&error).unwrap();
        for key in ["agentLabel", "agentTemplateKey", "modelName"] {
            assert!(missing.get(key).is_none());
        }
        error.agent_label = Some("Turn Agent".into());
        error.agent_template_key = Some("chat.minimal".into());
        error.model_name = Some("turn-model".into());
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(value["agentLabel"], "Turn Agent");
        assert_eq!(value["agentTemplateKey"], "chat.minimal");
        assert_eq!(value["modelName"], "turn-model");
        assert!(value.get("agent_label").is_none());
        assert_eq!(serde_json::from_value::<AgentStreamErrorData>(value).unwrap(), error);
    }

    #[test]
    fn provider_diagnostic_uses_the_shared_camel_case_contract_without_aliases() {
        let mut diagnostic = ModelFailureDiagnostic::new(ModelFailureReason::InvalidKey);
        diagnostic.http_status = Some(401);
        diagnostic.provider_code = Some("invalid_api_key".into());
        diagnostic.provider_type = Some("authentication_error".into());
        diagnostic.provider_param = Some("headers.authorization".into());
        diagnostic.provider_id = Some("0190f5fe-7c00-7a00-8000-000000000071".into());
        diagnostic.model_name = Some("actual-request-model".into());
        diagnostic.endpoint = Some("https://api.example.test/v1/chat/completions".into());
        diagnostic.request_id = Some("safe-request-id".into());
        diagnostic.retry_after_ms = Some(1000);
        diagnostic.protocol = Some("openai.chat_text".into());
        diagnostic.auth_scheme = Some("bearer".into());
        diagnostic.content_type = Some("application/json".into());
        let mut error = AgentStreamErrorData::legacy("Safe fixed authentication failure", None);
        error.provider_diagnostic = Some(diagnostic);
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(value["providerDiagnostic"]["reason"], "invalid_key");
        assert_eq!(value["providerDiagnostic"]["httpStatus"], 401);
        assert_eq!(value["providerDiagnostic"]["providerParam"], "headers.authorization");
        assert_eq!(value["providerDiagnostic"]["providerId"], "0190f5fe-7c00-7a00-8000-000000000071");
        assert_eq!(value["providerDiagnostic"]["modelName"], "actual-request-model");
        assert!(value.get("provider_diagnostic").is_none());
        assert!(value["providerDiagnostic"].get("http_status").is_none());
        assert_eq!(serde_json::from_value::<AgentStreamErrorData>(value).unwrap(), error);
        let missing = AgentStreamErrorData::legacy("Existing failure", None);
        assert!(serde_json::to_value(missing).unwrap().get("providerDiagnostic").is_none());
    }

    #[test]
    fn image_unsupported_serde_roundtrip() {
        let code = AgentErrorCode::UserLlmProviderImageUnsupported;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"USER_LLM_PROVIDER_IMAGE_UNSUPPORTED\"");
        let back: AgentErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
    }

    #[test]
    fn unbacked_completion_serde_roundtrip() {
        let code = AgentErrorCode::UserLlmProviderUnbackedCompletion;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"USER_LLM_PROVIDER_UNBACKED_COMPLETION\"");
        let back: AgentErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
    }

    #[test]
    fn agent_session_inconsistent_serde_roundtrip() {
        let code = AgentErrorCode::NomifunAgentSessionInconsistent;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"NOMIFUN_AGENT_SESSION_INCONSISTENT\"");
        let back: AgentErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
    }

    #[test]
    fn tool_result_encoding_error_serde_roundtrip() {
        let code = AgentErrorCode::NomifunToolResultEncodingError;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"NOMIFUN_TOOL_RESULT_ENCODING_ERROR\"");
        let back: AgentErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
    }

    #[test]
    fn classified_error_serializes_as_public_contract() {
        let payload = AgentStreamErrorData::classified(
            "The model provider rejected the request",
            AgentErrorCode::UserLlmProviderAuthFailed,
            AgentErrorOwnership::UserLlmProvider,
            Some("Provider returned 401.".into()),
            false,
            false,
            None,
        );

        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["message"], "The model provider rejected the request");
        assert_eq!(json["code"], "USER_LLM_PROVIDER_AUTH_FAILED");
        assert_eq!(json["ownership"], "user_llm_provider");
        assert!(json.get("workspacePath").is_none());
        assert!(json.get("taskIncompleteReason").is_none());
        assert_eq!(json["retryable"], false);
        assert_eq!(json["feedback_recommended"], false);
        assert!(json.get("resolution").is_none());
    }

    #[test]
    fn classified_error_serializes_resolution() {
        let payload = AgentStreamErrorData::classified(
            "The current response is still running",
            AgentErrorCode::NomifunConversationBusy,
            AgentErrorOwnership::Nomifun,
            Some("Conflict: Conversation is already processing a message".into()),
            true,
            false,
            Some(AgentErrorResolution::new(
                AgentErrorResolutionKind::WaitForCurrentResponse,
                Some(AgentErrorResolutionTarget::NewConversation),
            )),
        );

        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["code"], "NOMIFUN_CONVERSATION_BUSY");
        assert_eq!(json["resolution"]["kind"], "wait_for_current_response");
        assert_eq!(json["resolution"]["target"], "new_conversation");
    }

    #[test]
    fn legacy_error_payload_deserializes() {
        let json = serde_json::json!({
            "message": "legacy failure",
            "code": "UNKNOWN_UPSTREAM_ERROR"
        });

        let payload: AgentStreamErrorData = serde_json::from_value(json).unwrap();
        assert_eq!(payload.message, "legacy failure");
        assert_eq!(payload.code, Some(AgentErrorCode::UnknownUpstreamError));
        assert_eq!(payload.ownership, None);
        assert_eq!(payload.workspace_path, None);
        assert_eq!(payload.task_incomplete_reason, None);
        assert_eq!(payload.retryable, None);
        assert_eq!(payload.feedback_recommended, None);
    }

    #[test]
    fn legacy_error_payload_has_no_resolution() {
        let json = serde_json::json!({
            "message": "legacy failure",
            "code": "UNKNOWN_UPSTREAM_ERROR"
        });

        let payload: AgentStreamErrorData = serde_json::from_value(json).unwrap();
        assert_eq!(payload.resolution, None);
    }

    #[test]
    fn workspace_path_field_uses_camel_case_wire_key_and_accepts_legacy_snake_case() {
        let payload = AgentStreamErrorData {
            message: "workspace path rejected".into(),
            code: Some(AgentErrorCode::WorkspacePathEdgeWhitespaceRuntimeUnsupported),
            ownership: Some(AgentErrorOwnership::Nomifun),
            detail: Some("workspace detail".into()),
            workspace_path: Some("/tmp/Archive ".into()),
            agent_label: None,
            agent_template_key: None,
            model_name: None,
            task_incomplete_reason: None,
            provider_diagnostic: None,
            retryable: Some(false),
            feedback_recommended: Some(false),
            resolution: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json["code"],
            "WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED"
        );
        assert_eq!(json["workspacePath"], "/tmp/Archive ");
        assert!(
            json.get("workspace_path").is_none(),
            "new payloads must use the frontend camelCase contract"
        );

        let roundtrip: AgentStreamErrorData = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.workspace_path.as_deref(), Some("/tmp/Archive "));

        let legacy: AgentStreamErrorData = serde_json::from_value(serde_json::json!({
            "message": "legacy",
            "workspace_path": "/tmp/Legacy "
        }))
        .unwrap();
        assert_eq!(legacy.workspace_path.as_deref(), Some("/tmp/Legacy "));
    }

    #[test]
    fn task_incomplete_reason_roundtrips_as_camel_case_public_metadata() {
        for (reason, wire_value) in [
            (AgentTaskIncompleteReason::BlockedWork, "blocked_work"),
            (AgentTaskIncompleteReason::StepLimit, "step_limit"),
            (AgentTaskIncompleteReason::OutputTruncated, "output_truncated"),
            (AgentTaskIncompleteReason::UnresolvedPlan, "unresolved_plan"),
            (AgentTaskIncompleteReason::UnverifiedChanges, "unverified_changes"),
            (AgentTaskIncompleteReason::RunningProcesses, "running_processes"),
            (AgentTaskIncompleteReason::UnverifiedCompletion, "unverified_completion"),
            (AgentTaskIncompleteReason::RejectedControl, "rejected_control"),
            (AgentTaskIncompleteReason::NoProgress, "no_progress"),
            (AgentTaskIncompleteReason::ExecutionGuard, "execution_guard"),
            (AgentTaskIncompleteReason::RecoveryGuard, "recovery_guard"),
        ] {
            let mut payload = AgentStreamErrorData::classified(
                "The Agent stopped before completing the task",
                AgentErrorCode::NomifunTaskIncomplete,
                AgentErrorOwnership::Nomifun,
                Some("Original engine diagnostic".into()),
                false,
                false,
                None,
            );
            payload.task_incomplete_reason = Some(reason);
            let json = serde_json::to_value(&payload).unwrap();
            assert_eq!(json["taskIncompleteReason"], wire_value);
            assert!(json.get("task_incomplete_reason").is_none());
            let roundtrip: AgentStreamErrorData = serde_json::from_value(json).unwrap();
            assert_eq!(roundtrip, payload);
        }
    }
}
