use nomifun_api_types::{
    AgentErrorCode, AgentErrorOwnership, AgentErrorResolution, AgentErrorResolutionKind, AgentErrorResolutionTarget,
    AgentStreamErrorData, AgentFailureStage, AgentTaskIncompleteReason, ModelFailureDiagnostic, ModelFailureReason,
};
use nomifun_common::AppError;
use nomifun_net::secret_redaction::redact_url_queries;

const MAX_DETAIL_CHARS: usize = 1000;

#[derive(Debug, Clone)]
pub struct AgentSendError {
    stream_error: AgentStreamErrorData,
}

impl AgentSendError {
    pub(crate) fn invocation_code(kind: nomifun_agent_contracts::ModelInvocationFailureKind)
        -> nomifun_chat_model_broker::ChatModelErrorCode {
        use nomifun_agent_contracts::ModelInvocationFailureKind as K;
        use nomifun_chat_model_broker::ChatModelErrorCode as C;
        match kind {
            K::Auth => C::AuthenticationFailed,
            K::RateLimited => C::RateLimited,
            K::UnsupportedTask | K::ContentPolicy | K::NotPollable => C::UnsupportedFeature,
            K::NoAdapter => C::AdapterUnavailable,
            K::MissingConnection | K::Config => C::RouteNotFound,
            K::InvalidParams => C::InvalidRequest,
            K::ParseError | K::NonApiResponse => C::ProtocolViolation,
            K::QuotaExhausted | K::ProviderError | K::JobFailed | K::Network | K::Timeout => C::ProviderUnavailable,
        }
    }

    fn from_invocation_failure(kind: nomifun_agent_contracts::ModelInvocationFailureKind,
        message: &str, diagnostic: &ModelFailureDiagnostic) -> Self {
        let mut error = Self::from_model_failure(Self::invocation_code(kind),Some(diagnostic));
        if diagnostic.provider_detail.is_none() && diagnostic.transport_detail.is_none() {
            // This owner boundary carries already credential-redacted invocation
            // diagnostics. Keep them visible without treating prose as an envelope.
            error.stream_error.detail = Some(sanitize_error_detail(message));
        }
        error
    }

    /// The broker owns this classification. Provider prose cannot select an
    /// Agent code, and a finished Turn is never permission to replay effects.
    pub fn from_model_failure(
        code: nomifun_chat_model_broker::ChatModelErrorCode,
        diagnostic: Option<&ModelFailureDiagnostic>,
    ) -> Self {
        use nomifun_chat_model_broker::ChatModelErrorCode;
        let (mut message, mut agent_code, mut ownership, mut kind, mut target) = match code {
            ChatModelErrorCode::AuthenticationFailed => (
                "The model provider rejected authentication", AgentErrorCode::UserLlmProviderAuthFailed,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::CheckProviderCredentials,
                Some(AgentErrorResolutionTarget::ProviderSettings),
            ),
            ChatModelErrorCode::RateLimited => (
                "The model provider rate limit was reached", AgentErrorCode::UserLlmProviderRateLimited,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::Retry, None,
            ),
            ChatModelErrorCode::PromptTooLong => (
                "The model context limit was exceeded", AgentErrorCode::UserLlmProviderContextTooLarge,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::ReduceContext, None,
            ),
            ChatModelErrorCode::ProviderUnavailable => (
                "The model service is temporarily unavailable", AgentErrorCode::UserLlmProviderUnavailable,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::Retry, None,
            ),
            ChatModelErrorCode::InvalidRequest => (
                "The model provider rejected the request", AgentErrorCode::UserLlmProviderInvalidRequest,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::ChangeModel,
                Some(AgentErrorResolutionTarget::ProviderSettings),
            ),
            ChatModelErrorCode::UnsupportedFeature => (
                "The selected model does not support a required feature", AgentErrorCode::UserLlmProviderUnsupportedFeature,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::ChangeModel,
                Some(AgentErrorResolutionTarget::ProviderSettings),
            ),
            ChatModelErrorCode::ProtocolViolation => (
                "The model response did not match the expected protocol", AgentErrorCode::UserLlmProviderInvalidResponse,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::ChangeModel,
                Some(AgentErrorResolutionTarget::ProviderSettings),
            ),
            ChatModelErrorCode::StreamInterrupted | ChatModelErrorCode::Cancelled => (
                "The model response was interrupted", AgentErrorCode::UserLlmProviderStreamInterrupted,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::Retry, None,
            ),
            ChatModelErrorCode::RouteNotFound | ChatModelErrorCode::CredentialReferenceMissing
            | ChatModelErrorCode::CredentialTargetMismatch => (
                "The model provider configuration is unavailable", AgentErrorCode::UserLlmProviderConfigError,
                AgentErrorOwnership::UserLlmProvider, AgentErrorResolutionKind::CheckProviderCredentials,
                Some(AgentErrorResolutionTarget::ProviderSettings),
            ),
            ChatModelErrorCode::RouteRevisionMismatch => (
                "The model configuration no longer matches this Session", AgentErrorCode::NomifunSessionConfigurationChanged,
                AgentErrorOwnership::Nomifun, AgentErrorResolutionKind::StartNewSession,
                Some(AgentErrorResolutionTarget::NewConversation),
            ),
            ChatModelErrorCode::CausalityRejected | ChatModelErrorCode::DuplicateOperation
            | ChatModelErrorCode::ShadowNotPrimary | ChatModelErrorCode::SessionTerminal => (
                "The model request does not match the active Turn", AgentErrorCode::NomifunStateInconsistent,
                AgentErrorOwnership::Nomifun, AgentErrorResolutionKind::SendFeedback,
                Some(AgentErrorResolutionTarget::Feedback),
            ),
            ChatModelErrorCode::AdapterUnavailable | ChatModelErrorCode::Internal => (
                "The model request could not be executed", AgentErrorCode::NomifunInternalError,
                AgentErrorOwnership::Nomifun, AgentErrorResolutionKind::SendFeedback,
                Some(AgentErrorResolutionTarget::Feedback),
            ),
        };
        let synthesized;
        let diagnostic = if diagnostic.is_some() { diagnostic } else {
            synthesized = match code {
                ChatModelErrorCode::CredentialReferenceMissing => Some(ModelFailureDiagnostic::new(ModelFailureReason::CredentialsMissing)),
                ChatModelErrorCode::CredentialTargetMismatch => Some(ModelFailureDiagnostic::new(ModelFailureReason::CredentialTargetMismatch)),
                ChatModelErrorCode::AdapterUnavailable => Some(ModelFailureDiagnostic::new(ModelFailureReason::ConfigurationError)),
                _ => None,
            };
            synthesized.as_ref()
        };
        if let Some(diagnostic) = diagnostic {
            use ModelFailureReason as R;
            let local_application_failure = ownership == AgentErrorOwnership::Nomifun;
            let refined = match diagnostic.reason {
                R::ConfigurationError => (
                    "The local model configuration could not be executed", if local_application_failure { agent_code } else { AgentErrorCode::UserLlmProviderConfigError },
                    if local_application_failure { kind } else { AgentErrorResolutionKind::CheckProviderBaseUrl },
                    if local_application_failure { target } else { Some(AgentErrorResolutionTarget::ProviderSettings) },
                ),
                R::CredentialsMissing | R::CredentialTargetMismatch => (
                    "The selected model credentials are missing or do not match the configured provider", AgentErrorCode::UserLlmProviderConfigError,
                    AgentErrorResolutionKind::CheckProviderCredentials, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::InvalidEndpoint => (
                    "The configured API endpoint is invalid", AgentErrorCode::UserLlmProviderConfigError,
                    AgentErrorResolutionKind::CheckProviderBaseUrl, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::AuthFailed | R::InvalidKey | R::ExpiredKey => (
                    "The model provider rejected authentication", AgentErrorCode::UserLlmProviderAuthFailed,
                    AgentErrorResolutionKind::CheckProviderCredentials, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::BillingRequired | R::InsufficientQuota | R::InsufficientBalance | R::SubscriptionExpired | R::ModelNotInPlan | R::SpendLimitReached => (
                    "The model provider account cannot cover this request", AgentErrorCode::UserLlmProviderBillingRequired,
                    AgentErrorResolutionKind::CheckProviderBilling, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::PermissionDenied | R::ModelPermissionDenied => (
                    "The model provider denied access", AgentErrorCode::UserLlmProviderPermissionDenied,
                    AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::ModelNotFound => (
                    "The selected model was not found", AgentErrorCode::UserLlmProviderModelNotFound,
                    AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::EndpointMissing => (
                    "The requested API endpoint was not found", AgentErrorCode::UserLlmProviderEndpointNotFound,
                    AgentErrorResolutionKind::CheckProviderBaseUrl, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::NonApiResponse => (
                    "The endpoint returned a non-API response", AgentErrorCode::UserLlmProviderInvalidResponse,
                    AgentErrorResolutionKind::CheckProviderBaseUrl, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::AuthSchemeMismatch => (
                    "The configured authentication scheme was not accepted", AgentErrorCode::UserLlmProviderConfigError,
                    AgentErrorResolutionKind::CheckProviderCredentials, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::DnsFailure | R::ConnectionFailed | R::TlsFailure | R::ProxyFailure | R::NetworkFailure => (
                    "The connection to the model service failed", AgentErrorCode::UserLlmProviderNetworkError,
                    AgentErrorResolutionKind::CheckProviderBaseUrl, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::RequestTimeout => (
                    "The model request timed out", AgentErrorCode::UserLlmProviderTimeout,
                    AgentErrorResolutionKind::Retry, None,
                ),
                R::PromptTooLong => (
                    "The request exceeded the model context limit", AgentErrorCode::UserLlmProviderContextTooLarge,
                    AgentErrorResolutionKind::ReduceContext, None,
                ),
                R::UpstreamServerError | R::ProviderOverloaded => (
                    "The model service returned a server error", AgentErrorCode::UserLlmProviderGatewayError,
                    AgentErrorResolutionKind::Retry, None,
                ),
                R::ProviderUnavailable => (
                    "The model service is unavailable", AgentErrorCode::UserLlmProviderUnavailable,
                    AgentErrorResolutionKind::Retry, None,
                ),
                R::RateLimited => (
                    "The model service temporarily limited requests", AgentErrorCode::UserLlmProviderRateLimited,
                    AgentErrorResolutionKind::Retry, None,
                ),
                R::StreamInterrupted => (
                    "The model response was interrupted", AgentErrorCode::UserLlmProviderStreamInterrupted,
                    AgentErrorResolutionKind::Retry, None,
                ),
                R::InvalidResponse => (
                    "The model service returned an invalid response", AgentErrorCode::UserLlmProviderInvalidResponse,
                    AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::InvalidRequest | R::ContentPolicy => (
                    "The model service rejected the request", AgentErrorCode::UserLlmProviderInvalidRequest,
                    AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
                R::UnsupportedFeature => (
                    "The selected model does not support the required feature", AgentErrorCode::UserLlmProviderUnsupportedFeature,
                    AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings),
                ),
            };
            (message, agent_code, kind, target) = refined;
            ownership = if local_application_failure && diagnostic.reason == R::ConfigurationError {
                AgentErrorOwnership::Nomifun
            } else { AgentErrorOwnership::UserLlmProvider };
        }
        let resolution = if diagnostic.is_some_and(|diagnostic| diagnostic.reason == ModelFailureReason::ContentPolicy) {
            None
        } else { resolution(kind, target) };
        // Arbitrary provider prose is not approved technical detail. Transport
        // causes arrive in the same credential-redacted typed diagnostic.
        let mut error = Self::new(message, agent_code, ownership, None, false,
            ownership == AgentErrorOwnership::Nomifun, resolution);
        error.stream_error.provider_diagnostic = diagnostic.cloned();
        error.stream_error.failure_stage = Some(if matches!(code,
            ChatModelErrorCode::RouteNotFound | ChatModelErrorCode::CredentialReferenceMissing
            | ChatModelErrorCode::CredentialTargetMismatch | ChatModelErrorCode::RouteRevisionMismatch
            | ChatModelErrorCode::AdapterUnavailable) {
            AgentFailureStage::Preparation
        } else { AgentFailureStage::ModelResponse });
        error
    }

    pub fn from_runtime_turn_failure(
        detail: impl Into<String>,
        failure: Option<&nomifun_agent_runtime::AgentTurnFailure>,
    ) -> Self {
        use nomifun_agent_runtime::AgentTurnFailure;
        let detail = detail.into();
        match failure {
            Some(AgentTurnFailure::SystemExecution { diagnostic }) => Self::from_system_execution_failure(diagnostic),
            Some(AgentTurnFailure::Model { code, diagnostic }) => Self::from_model_failure(*code, diagnostic.as_ref()),
            Some(AgentTurnFailure::ModelInvocation { kind, diagnostic }) => Self::from_invocation_failure(*kind,&detail,diagnostic),
            Some(AgentTurnFailure::ModelStreamEndedWithoutTerminal) => Self::new(
                "The model response ended before completion was confirmed", AgentErrorCode::UserLlmProviderStreamInterrupted,
                AgentErrorOwnership::UserLlmProvider, Some(detail), false, false,
                resolution(AgentErrorResolutionKind::Retry, None),
            ).at_stage(AgentFailureStage::ModelResponse),
            Some(AgentTurnFailure::InvalidModelEvent) => Self::new(
                "The model returned an invalid response", AgentErrorCode::UserLlmProviderInvalidResponse,
                AgentErrorOwnership::UserLlmProvider, Some(detail), false, false,
                resolution(AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings)),
            ).at_stage(AgentFailureStage::ModelResponse),
            Some(AgentTurnFailure::TaskIncomplete { reason }) => Self::task_incomplete(*reason, detail),
            Some(AgentTurnFailure::Runtime { stage }) => Self::runtime_failure(*stage, detail),
            Some(AgentTurnFailure::ResourceBindingIncomplete { diagnostic }) => Self::resource_binding_failure(diagnostic, detail),
            Some(AgentTurnFailure::SessionConfigurationChanged) => Self::session_configuration_changed(detail),
            Some(AgentTurnFailure::WorkspacePathUnsupported { path }) => Self::workspace_failure(
                AgentErrorCode::WorkspacePathEdgeWhitespaceRuntimeUnsupported, path, detail),
            Some(AgentTurnFailure::WorkspaceDirectoryUnavailable { path }) => Self::workspace_failure(
                AgentErrorCode::WorkspaceDirectoryRuntimeUnavailable, path, detail),
            Some(AgentTurnFailure::HostPermissionDenied) => Self::permission_failure(detail),
            None => Self::source_unrecorded(detail),
        }
    }

    /// Missing provenance is evidence missing from this fact, not a provider fault.
    pub fn source_unrecorded(detail: impl Into<String>) -> Self {
        let mut error = AgentStreamErrorData::legacy(
            "The failure source was not recorded", Some(AgentErrorCode::AgentFailureSourceUnrecorded));
        error.detail = Some(sanitize_error_detail(&detail.into()));
        error.retryable = Some(false);
        error.feedback_recommended = Some(true);
        Self { stream_error: error }
    }

    pub fn runtime_failure(stage: AgentFailureStage, detail: impl Into<String>) -> Self {
        Self::new("The local Agent runtime could not complete this operation", AgentErrorCode::NomifunRuntimeFailed,
            AgentErrorOwnership::Nomifun, Some(detail.into()), false, true, None).at_stage(stage)
    }

    pub fn task_incomplete(reason: AgentTaskIncompleteReason, detail: impl Into<String>) -> Self {
        use AgentTaskIncompleteReason as R;
        let stage = match reason {
            R::StepLimit | R::OutputTruncated => AgentFailureStage::ModelResponse,
            R::RejectedControl | R::NoProgress | R::ExecutionGuard => AgentFailureStage::ToolExecution,
            R::RecoveryGuard => AgentFailureStage::Recovery,
            R::BlockedWork | R::UnresolvedPlan | R::UnverifiedChanges | R::RunningProcesses
            | R::UnverifiedCompletion | R::CompletionReportRejected => AgentFailureStage::CompletionReport,
        };
        let mut error = Self::new("The Agent stopped before completing the task", AgentErrorCode::NomifunTaskIncomplete,
            AgentErrorOwnership::Nomifun, Some(detail.into()), false, false, None).at_stage(stage);
        error.stream_error.task_incomplete_reason = Some(reason);
        error
    }

    fn at_stage(mut self, stage: AgentFailureStage) -> Self {
        self.stream_error.failure_stage = Some(stage);
        self
    }

    fn resource_binding_failure(diagnostic: &nomifun_api_types::AgentResourceBindingFailure, detail: String) -> Self {
        let mut error = Self::new("The Agent's required resources were not prepared",
            AgentErrorCode::NomifunAgentResourceBindingIncomplete, AgentErrorOwnership::Nomifun,
            Some(detail), false, true, None).at_stage(AgentFailureStage::Preparation);
        error.stream_error.resource_binding_failure = Some(diagnostic.clone());
        error
    }

    fn workspace_failure(code: AgentErrorCode, path: &str, detail: String) -> Self {
        let mut error = Self::new("The workspace is not available for execution", code,
            AgentErrorOwnership::Nomifun, Some(detail), false, false,
            resolution(AgentErrorResolutionKind::StartNewSession, Some(AgentErrorResolutionTarget::NewConversation)))
            .at_stage(AgentFailureStage::Preparation);
        error.stream_error.workspace_path = Some(path.into());
        error
    }

    fn permission_failure(detail: String) -> Self {
        Self::new("The local host refused this operation", AgentErrorCode::NomifunPermissionError,
            AgentErrorOwnership::Nomifun, Some(detail), false, false, None)
    }

    /// Current canonical machine reasons only; manual pauses and unrelated
    /// recovery/cleanup suspensions cannot become failures through this mapper.
    pub fn from_model_pause_reason(reason: &str) -> Option<Self> {
        use nomifun_agent_runtime::AgentTurnFailure;
        let suffix = reason.strip_prefix("EXECUTION_MODEL_")?;
        let failure = match suffix {
            "STREAM_ENDED_WITHOUT_TERMINAL" => AgentTurnFailure::ModelStreamEndedWithoutTerminal,
            "INVALID_EVENT" => AgentTurnFailure::InvalidModelEvent,
            _ => AgentTurnFailure::Model { code: serde_json::from_value(serde_json::json!(suffix)).ok()?, diagnostic: None },
        };
        let mut error = Self::from_runtime_turn_failure(reason, Some(&failure));
        error.stream_error.detail = Some(reason.into());
        // An old pause records only its coarse machine cause, not today's
        // credential/configuration evidence or a fine provider diagnosis.
        error.stream_error.provider_diagnostic = None;
        Some(error)
    }

    pub fn session_configuration_changed(detail: impl Into<String>) -> Self {
        Self::new(
            "The tool configuration no longer matches this session",
            AgentErrorCode::NomifunSessionConfigurationChanged,
            AgentErrorOwnership::Nomifun,
            Some(detail.into()),
            false,
            false,
            resolution(AgentErrorResolutionKind::StartNewSession,
                Some(AgentErrorResolutionTarget::NewConversation)),
        ).at_stage(AgentFailureStage::Preparation)
    }

    /// An unrecoverable loss of the manager's permanent event relay. This is
    /// deliberately distinct from provider/API errors: Conversation evicts
    /// the cached runtime on this code before admitting another turn.
    pub fn stream_broken(detail: impl Into<String>) -> Self {
        Self::new(
            "The Agent event stream disconnected",
            AgentErrorCode::NomifunStreamBroken,
            AgentErrorOwnership::Nomifun,
            Some(detail.into()),
            true,
            true,
            resolution(
                AgentErrorResolutionKind::Retry,
                Some(AgentErrorResolutionTarget::AgentSettings),
            ),
        )
    }

    pub fn new(
        message: impl Into<String>,
        code: AgentErrorCode,
        ownership: AgentErrorOwnership,
        detail: Option<String>,
        retryable: bool,
        feedback_recommended: bool,
        resolution: Option<AgentErrorResolution>,
    ) -> Self {
        Self {
            stream_error: AgentStreamErrorData::classified(
                message,
                code,
                ownership,
                detail.map(|d| sanitize_error_detail(&d)),
                retryable,
                feedback_recommended,
                resolution,
            ),
        }
    }

    pub fn from_app_error(err: AppError) -> Self {
        Self::from_app_error_ref(&err)
    }

    pub fn from_app_error_ref(err: &AppError) -> Self {
        let detail = err.to_string();
        match err {
            AppError::ModelInvocation { kind, message, diagnostic } => {
                Self::from_invocation_failure(*kind, message, diagnostic)
            }
            AppError::SystemExecution(failure) => Self::from_system_execution_failure(failure),
            AppError::SessionConfigurationChanged(_) => Self::session_configuration_changed(detail),
            AppError::AgentResourceBindingIncomplete { missing_resource_kinds, missing_resource_operations, .. } =>
                Self::resource_binding_failure(&nomifun_api_types::AgentResourceBindingFailure {
                    missing_resource_kinds: missing_resource_kinds.clone(),
                    missing_resource_operations: missing_resource_operations.clone(),
                }, detail),
            AppError::WorkspacePathEdgeWhitespaceRuntimeUnsupported(path) => Self::workspace_failure(
                AgentErrorCode::WorkspacePathEdgeWhitespaceRuntimeUnsupported, path, detail),
            AppError::WorkspaceDirectoryRuntimeUnavailable(path) => Self::workspace_failure(
                AgentErrorCode::WorkspaceDirectoryRuntimeUnavailable, path, detail),
            AppError::Forbidden(_) | AppError::Unauthorized(_) | AppError::NativeSystemAccessDenied { .. } => Self::permission_failure(detail),
            AppError::DatabaseUnavailable { .. } => Self::runtime_failure(AgentFailureStage::Persistence, detail),
            AppError::Conflict(_) | AppError::RevisionConflict(_) => Self::new(
                "The request conflicts with the current Agent state", AgentErrorCode::NomifunStateInconsistent,
                AgentErrorOwnership::Nomifun, Some(detail), false, true, None),
            AppError::Internal(_) => Self::new("The local host could not execute this operation",
                AgentErrorCode::NomifunInternalError, AgentErrorOwnership::Nomifun, Some(detail), false, true, None),
            // HTTP wrappers alone do not prove that the model provider failed.
            // The owner must provide ModelInvocation or another typed cause.
            AppError::BadGateway(_) | AppError::Timeout(_) | AppError::RateLimited => Self::source_unrecorded(detail),
            _ => Self::runtime_failure(AgentFailureStage::Preparation, detail),
        }
    }

    pub fn stream_error(&self) -> &AgentStreamErrorData {
        &self.stream_error
    }

    fn from_system_execution_failure(failure: &nomifun_agent_contracts::SystemExecutionFailure) -> Self {
        let code = match failure.code.as_str() {
            "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED" => AgentErrorCode::SystemExecutionAuthenticationCancelled,
            "SYSTEM_EXECUTION_AUTHENTICATION_DENIED" => AgentErrorCode::SystemExecutionAuthenticationDenied,
            "SYSTEM_EXECUTION_AUTHENTICATION_TIMED_OUT" => AgentErrorCode::SystemExecutionAuthenticationTimedOut,
            "SYSTEM_EXECUTION_AUTHENTICATION_UNAVAILABLE" => AgentErrorCode::SystemExecutionAuthenticationUnavailable,
            "SYSTEM_EXECUTION_APPROVAL_REQUIRED" => AgentErrorCode::SystemExecutionApprovalRequired,
            "SYSTEM_EXECUTION_NOT_AVAILABLE" => AgentErrorCode::SystemExecutionNotAvailable,
            "SYSTEM_EXECUTION_RESULT_UNKNOWN" | "system_result_unknown" => AgentErrorCode::SystemExecutionResultUnknown,
            "SYSTEM_READ_RESULT_UNKNOWN" => AgentErrorCode::SystemReadResultUnknown,
            _ => AgentErrorCode::SystemExecutionFailed,
        };
        let mut error = Self::new("Native system execution could not complete", code,
            AgentErrorOwnership::Nomifun, None, false, false, None);
        let mut diagnostic = failure.clone();
        diagnostic.message = sanitize_error_detail(&diagnostic.message);
        error.stream_error.system_execution_error = Some(diagnostic);
        error.at_stage(AgentFailureStage::ToolExecution)
    }

    pub fn into_stream_error(self) -> AgentStreamErrorData {
        self.stream_error
    }

    pub fn code(&self) -> Option<AgentErrorCode> {
        self.stream_error.code
    }

    pub fn ownership(&self) -> Option<AgentErrorOwnership> {
        self.stream_error.ownership
    }
}

impl std::fmt::Display for AgentSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.stream_error.message)
    }
}

impl std::error::Error for AgentSendError {}

impl From<AppError> for AgentSendError {
    fn from(err: AppError) -> Self {
        Self::from_app_error(err)
    }
}

fn resolution(
    kind: AgentErrorResolutionKind,
    target: Option<AgentErrorResolutionTarget>,
) -> Option<AgentErrorResolution> {
    Some(AgentErrorResolution::new(kind, target))
}

pub(crate) fn sanitize_error_detail(input: &str) -> String {
    let stripped = strip_markup(input);
    let without_query = redact_url_queries(&stripped);
    let redacted = redact_lines(&without_query);
    truncate_chars(redacted.trim(), MAX_DETAIL_CHARS)
}

fn strip_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match (ch, in_tag) {
            ('<', _) => in_tag = true,
            ('>', true) => {
                in_tag = false;
                out.push(' ');
            }
            (c, false) => out.push(c),
            _ => {}
        }
    }
    out
}

fn redact_lines(input: &str) -> String {
    let mut out = String::new();
    for line in input.lines() {
        let cleaned = if is_sensitive_header_line(line) {
            "<redacted header>".to_owned()
        } else {
            redact_secret_words(line)
        };
        push_bounded_line(&mut out, &cleaned);
        if out.chars().count() >= MAX_DETAIL_CHARS {
            break;
        }
    }
    out
}

fn push_bounded_line(out: &mut String, line: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
}

fn is_sensitive_header_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("authorization:")
        || lower.contains("x-api-key:")
        || lower.contains("api-key:")
        || lower.contains("api_key:")
}

fn redact_secret_words(line: &str) -> String {
    let mut redact_bearer_value = false;
    line.split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let follows_bearer = redact_bearer_value;
            // Whitespace has already been consumed by the iterator. The
            // credential is the next token, not a suffix of this one.
            redact_bearer_value = lower.trim_matches(['"', '\'', '(', ')', '[', ']']) == "bearer";
            if follows_bearer
                || lower.starts_with("sk-")
                || lower.contains("api_key=")
                || lower.contains("apikey=")
                || lower.contains("access_token=")
                || lower.contains("token=")
            {
                "<redacted>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() > max {
        let mut out: String = value.chars().take(max.saturating_sub(3)).collect();
        out.extend("...".chars().take(max.min(3)));
        out
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_host_resources_are_typed_local_and_non_retryable() {
        let error = AgentSendError::from_app_error(AppError::AgentResourceBindingIncomplete {
            message: "Host admission refused before provider dispatch".into(),
            missing_resource_kinds: vec!["workspace".into(), "process_session".into()],
            missing_resource_operations: std::collections::BTreeMap::from([
                ("workspace".into(), std::collections::BTreeSet::from(["read".into(), "write".into()])),
            ]),
        });
        let data = error.stream_error();
        assert_eq!(data.code, Some(AgentErrorCode::NomifunAgentResourceBindingIncomplete));
        assert_eq!(data.ownership, Some(AgentErrorOwnership::Nomifun));
        assert_eq!(data.retryable, Some(false));
        assert_eq!(data.resource_binding_failure.as_ref().unwrap().missing_resource_kinds,
            ["workspace", "process_session"]);
        let provider = AgentSendError::from_app_error(AppError::BadGateway(
            "NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE: workspace grant".into()));
        assert!(provider.stream_error().resource_binding_failure.is_none());
        assert_ne!(provider.code(), data.code);
    }

    #[test]
    fn native_authentication_codes_are_preserved_without_reclassifying_binding_acl_or_unknown() {
        for code in ["SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED", "SYSTEM_EXECUTION_AUTHENTICATION_DENIED",
            "SYSTEM_EXECUTION_AUTHENTICATION_TIMED_OUT", "SYSTEM_EXECUTION_AUTHENTICATION_UNAVAILABLE",
            "SYSTEM_EXECUTION_APPROVAL_REQUIRED", "SYSTEM_EXECUTION_NOT_AVAILABLE", "SYSTEM_EXECUTION_RESULT_UNKNOWN", "SYSTEM_READ_RESULT_UNKNOWN"] {
            let failure = nomifun_agent_contracts::SystemExecutionFailure::from_owner(code.into(), "PRESET_RESOURCE_NOT_BOUND; raw_os_error=1223 is only text".into(), &serde_json::json!({}));
            let error = AgentSendError::from_runtime_turn_failure(failure.message.clone(),
                Some(&nomifun_agent_runtime::AgentTurnFailure::SystemExecution { diagnostic:failure.clone() }));
            let api = error.stream_error();
            assert_eq!(serde_json::to_value(api.code).unwrap(), code);
            assert_eq!(api.system_execution_error.as_ref(), Some(&failure));
            assert_eq!(api.retryable, Some(false));
            assert!(api.provider_diagnostic.is_none() && api.resolution.is_none());
            assert_eq!(api.system_execution_error.as_ref().unwrap().raw_os_error, None);
            let from_app = AgentSendError::from_app_error(AppError::SystemExecution(failure));
            assert_eq!(from_app.stream_error(), api);
        }
        assert_eq!(AgentSendError::from_app_error(AppError::Forbidden("real missing resource binding".into())).code(), Some(AgentErrorCode::NomifunPermissionError));
        assert!(AgentSendError::from_app_error(AppError::NativeSystemAccessDenied { operation:"ordinary ACL".into(), source:std::io::Error::from_raw_os_error(5) }).stream_error().system_execution_error.is_none());
        assert!(AgentSendError::from_app_error(AppError::Internal("SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED raw_os_error=1223".into())).stream_error().system_execution_error.is_none());
    }

    #[test]
    fn typed_model_failure_never_guesses_an_agent_code_from_provider_prose() {
        use nomifun_chat_model_broker::ChatModelErrorCode;
        for (broker, expected) in [
            (ChatModelErrorCode::AuthenticationFailed, AgentErrorCode::UserLlmProviderAuthFailed),
            (ChatModelErrorCode::RateLimited, AgentErrorCode::UserLlmProviderRateLimited),
            (ChatModelErrorCode::PromptTooLong, AgentErrorCode::UserLlmProviderContextTooLarge),
            (ChatModelErrorCode::ProviderUnavailable, AgentErrorCode::UserLlmProviderUnavailable),
            (ChatModelErrorCode::UnsupportedFeature, AgentErrorCode::UserLlmProviderUnsupportedFeature),
            (ChatModelErrorCode::ProtocolViolation, AgentErrorCode::UserLlmProviderInvalidResponse),
            (ChatModelErrorCode::StreamInterrupted, AgentErrorCode::UserLlmProviderStreamInterrupted),
        ] {
            let error = AgentSendError::from_runtime_turn_failure("quota exceeded; authentication failed; unknown model", Some(
                &nomifun_agent_runtime::AgentTurnFailure::Model { code: broker, diagnostic: None }));
            assert_eq!(error.code(), Some(expected));
            assert!(error.stream_error().detail.is_none());
            assert!(!error.stream_error().message.contains("quota exceeded"));
            assert_eq!(error.stream_error().retryable, Some(false));
            assert!(error.stream_error().provider_diagnostic.is_none());
        }
    }

    #[test]
    fn current_model_pause_mapper_rejects_manual_cleanup_and_untyped_reasons() {
        for reason in ["EXECUTION_USER_REQUESTED", "EXECUTION_CLEANUP_UNPROVEN", "EXECUTION_PREPARATION_BLOCKED",
            "EXECUTION_MODEL_unknown", "EXECUTION_MODEL_PRIVATE_SECRET", "authentication failed"] {
            assert!(AgentSendError::from_model_pause_reason(reason).is_none(), "{reason}");
        }
        for (reason, expected) in [
            ("EXECUTION_MODEL_PROVIDER_UNAVAILABLE", AgentErrorCode::UserLlmProviderUnavailable),
            ("EXECUTION_MODEL_AUTHENTICATION_FAILED", AgentErrorCode::UserLlmProviderAuthFailed),
            ("EXECUTION_MODEL_ADAPTER_UNAVAILABLE", AgentErrorCode::NomifunInternalError),
            ("EXECUTION_MODEL_CREDENTIAL_REFERENCE_MISSING", AgentErrorCode::UserLlmProviderConfigError),
            ("EXECUTION_MODEL_STREAM_ENDED_WITHOUT_TERMINAL", AgentErrorCode::UserLlmProviderStreamInterrupted),
            ("EXECUTION_MODEL_INVALID_EVENT", AgentErrorCode::UserLlmProviderInvalidResponse),
        ] {
            let error = AgentSendError::from_model_pause_reason(reason).unwrap();
            assert_eq!(error.code(), Some(expected));
            assert_eq!(error.stream_error().detail.as_deref(), Some(reason));
            assert_eq!(error.stream_error().retryable, Some(false));
            assert!(error.stream_error().provider_diagnostic.is_none(), "a recorded pause does not contain today's fine evidence");
        }
    }
    #[test]
    fn typed_provider_reasons_select_precise_existing_codes_without_prose_inference() {
        use nomifun_chat_model_broker::ChatModelErrorCode;
        use ModelFailureReason as R;
        for (reason, expected) in [
            (R::AuthFailed, AgentErrorCode::UserLlmProviderAuthFailed),
            (R::InvalidKey, AgentErrorCode::UserLlmProviderAuthFailed),
            (R::ExpiredKey, AgentErrorCode::UserLlmProviderAuthFailed),
            (R::BillingRequired, AgentErrorCode::UserLlmProviderBillingRequired),
            (R::InsufficientQuota, AgentErrorCode::UserLlmProviderBillingRequired),
            (R::InsufficientBalance, AgentErrorCode::UserLlmProviderBillingRequired),
            (R::SubscriptionExpired, AgentErrorCode::UserLlmProviderBillingRequired),
            (R::ModelNotInPlan, AgentErrorCode::UserLlmProviderBillingRequired),
            (R::SpendLimitReached, AgentErrorCode::UserLlmProviderBillingRequired),
            (R::PermissionDenied, AgentErrorCode::UserLlmProviderPermissionDenied),
            (R::ModelPermissionDenied, AgentErrorCode::UserLlmProviderPermissionDenied),
            (R::ModelNotFound, AgentErrorCode::UserLlmProviderModelNotFound),
            (R::EndpointMissing, AgentErrorCode::UserLlmProviderEndpointNotFound),
            (R::NonApiResponse, AgentErrorCode::UserLlmProviderInvalidResponse),
            (R::AuthSchemeMismatch, AgentErrorCode::UserLlmProviderConfigError),
            (R::DnsFailure, AgentErrorCode::UserLlmProviderNetworkError),
            (R::ConnectionFailed, AgentErrorCode::UserLlmProviderNetworkError),
            (R::TlsFailure, AgentErrorCode::UserLlmProviderNetworkError),
            (R::ProxyFailure, AgentErrorCode::UserLlmProviderNetworkError),
            (R::NetworkFailure, AgentErrorCode::UserLlmProviderNetworkError),
            (R::RequestTimeout, AgentErrorCode::UserLlmProviderTimeout),
            (R::UpstreamServerError, AgentErrorCode::UserLlmProviderGatewayError),
            (R::ProviderOverloaded, AgentErrorCode::UserLlmProviderGatewayError),
            (R::ProviderUnavailable, AgentErrorCode::UserLlmProviderUnavailable),
            (R::RateLimited, AgentErrorCode::UserLlmProviderRateLimited),
            (R::StreamInterrupted, AgentErrorCode::UserLlmProviderStreamInterrupted),
            (R::InvalidResponse, AgentErrorCode::UserLlmProviderInvalidResponse),
            (R::InvalidRequest, AgentErrorCode::UserLlmProviderInvalidRequest),
            (R::UnsupportedFeature, AgentErrorCode::UserLlmProviderUnsupportedFeature),
            (R::ContentPolicy, AgentErrorCode::UserLlmProviderInvalidRequest),
            (R::PromptTooLong, AgentErrorCode::UserLlmProviderContextTooLarge),
            (R::ConfigurationError, AgentErrorCode::UserLlmProviderConfigError),
            (R::InvalidEndpoint, AgentErrorCode::UserLlmProviderConfigError),
            (R::CredentialsMissing, AgentErrorCode::UserLlmProviderConfigError),
            (R::CredentialTargetMismatch, AgentErrorCode::UserLlmProviderConfigError),
        ] {
            let mut diagnostic = ModelFailureDiagnostic::new(reason);
            diagnostic.http_status = Some(400);
            diagnostic.provider_code = Some("safe_machine_code".into());
            let failure = nomifun_agent_runtime::AgentTurnFailure::Model {
                code: ChatModelErrorCode::ProviderUnavailable, diagnostic: Some(diagnostic.clone()),
            };
            let error = AgentSendError::from_runtime_turn_failure("untrusted provider body containing a private credential", Some(&failure));
            assert_eq!(error.code(), Some(expected));
            assert_eq!(error.stream_error().provider_diagnostic, Some(diagnostic));
            assert_eq!(error.stream_error().retryable, Some(false));
            assert!(error.stream_error().detail.is_none());
            assert!(!error.stream_error().message.contains("private credential"));
            if reason == R::ContentPolicy { assert!(error.stream_error().resolution.is_none()); }
        }
    }

    #[test]
    fn exact_local_broker_faults_never_become_remote_request_rejections() {
        use nomifun_chat_model_broker::ChatModelErrorCode;
        for (code, reason, owner, public) in [
            (ChatModelErrorCode::CredentialReferenceMissing, ModelFailureReason::CredentialsMissing,
                AgentErrorOwnership::UserLlmProvider, AgentErrorCode::UserLlmProviderConfigError),
            (ChatModelErrorCode::CredentialTargetMismatch, ModelFailureReason::CredentialTargetMismatch,
                AgentErrorOwnership::UserLlmProvider, AgentErrorCode::UserLlmProviderConfigError),
            (ChatModelErrorCode::AdapterUnavailable, ModelFailureReason::ConfigurationError,
                AgentErrorOwnership::Nomifun, AgentErrorCode::NomifunInternalError),
        ] {
            let error = AgentSendError::from_model_failure(code, None);
            assert_eq!(error.code(), Some(public));
            assert_eq!(error.ownership(), Some(owner));
            let diagnostic = error.stream_error().provider_diagnostic.as_ref().unwrap();
            assert_eq!(diagnostic.reason, reason);
            assert!(diagnostic.http_status.is_none());
            assert!(diagnostic.endpoint.is_none());
            assert!(!error.stream_error().message.contains("rejected"));
        }
    }

    #[test]
    fn approved_transport_cause_survives_as_one_typed_diagnostic() {
        let mut diagnostic = ModelFailureDiagnostic::new(ModelFailureReason::ConnectionFailed);
        diagnostic.transport_detail = Some("connection refused (os error 61)".into());
        diagnostic.endpoint = Some("https://api.example.test/v1/chat/completions".into());
        let error = AgentSendError::from_model_failure(nomifun_chat_model_broker::ChatModelErrorCode::ProviderUnavailable, Some(&diagnostic));
        assert_eq!(error.stream_error().provider_diagnostic.as_ref(), Some(&diagnostic));
        assert!(error.stream_error().detail.is_none(), "the approved cause is not copied into a second raw-detail field");
    }

    #[test]
    fn sanitize_error_detail_redacts_bearer_values_after_tokenization() {
        for input in [
            "upstream replied: Bearer unknown-secret and failed",
            "upstream replied: bEaReR\tunknown-secret and failed",
            "<b>upstream replied:</b> Bearer unknown-secret and failed",
            "upstream replied: \"Bearer unknown-secret\" and failed",
            "upstream replied: (Bearer unknown-secret) and failed",
        ] {
            let detail = sanitize_error_detail(input);
            assert!(!detail.contains("unknown-secret"), "credential escaped: {detail}");
            assert!(detail.contains("and failed"));
            assert!(detail.contains("<redacted>"));
        }
    }

    #[test]
    fn bearer_without_a_value_does_not_consume_other_lines() {
        assert_eq!(sanitize_error_detail("Bearer\nconnection refused"), "Bearer\nconnection refused");
        assert_eq!(sanitize_error_detail("bearer-like ordinary message"), "bearer-like ordinary message");
    }

    #[test]
    fn local_typed_failures_keep_their_reason_regardless_of_diagnostic_words() {
        use nomifun_agent_runtime::AgentTurnFailure;
        let prose = "provider returned 401; timeout; model step limit; status delivered";
        let error = AgentSendError::from_runtime_turn_failure(prose, Some(&AgentTurnFailure::TaskIncomplete {
            reason: AgentTaskIncompleteReason::CompletionReportRejected,
        }));
        assert_eq!(error.code(), Some(AgentErrorCode::NomifunTaskIncomplete));
        assert_eq!(error.ownership(), Some(AgentErrorOwnership::Nomifun));
        assert_eq!(error.stream_error().task_incomplete_reason, Some(AgentTaskIncompleteReason::CompletionReportRejected));
        assert_eq!(error.stream_error().failure_stage, Some(AgentFailureStage::CompletionReport));
        assert_eq!(error.stream_error().detail.as_deref(), Some(prose));
        assert_eq!(error.stream_error().retryable, Some(false));
        assert!(error.stream_error().provider_diagnostic.is_none());
        let runtime = AgentSendError::from_runtime_turn_failure(prose, Some(&AgentTurnFailure::Runtime {
            stage: AgentFailureStage::Persistence,
        }));
        assert_eq!(runtime.code(), Some(AgentErrorCode::NomifunRuntimeFailed));
        assert_eq!(runtime.stream_error().failure_stage, Some(AgentFailureStage::Persistence));
    }

    #[test]
    fn absent_provenance_and_untyped_http_wrappers_do_not_guess_or_offer_replay() {
        let detail = "provider returned 401 invalid api key; model step limit of 32 exceeded";
        for error in [
            AgentSendError::from_runtime_turn_failure(detail, None),
            AgentSendError::from_app_error(AppError::BadGateway(detail.into())),
            AgentSendError::from_app_error(AppError::Timeout(detail.into())),
        ] {
            assert_eq!(error.code(), Some(AgentErrorCode::AgentFailureSourceUnrecorded));
            assert_eq!(error.ownership(), None);
            assert_eq!(error.stream_error().failure_stage, None);
            assert_eq!(error.stream_error().retryable, Some(false));
            assert!(error.stream_error().detail.as_ref().unwrap().contains("401"));
            assert!(error.stream_error().provider_diagnostic.is_none());
        }
        let conflict = AgentSendError::from_app_error(AppError::Conflict(detail.into()));
        assert_eq!(conflict.code(), Some(AgentErrorCode::NomifunStateInconsistent));
        assert_eq!(conflict.ownership(), Some(AgentErrorOwnership::Nomifun));
    }

    #[test]
    fn typed_invocation_diagnostic_survives_the_application_boundary() {
        let mut diagnostic = ModelFailureDiagnostic::new(ModelFailureReason::InsufficientBalance);
        diagnostic.http_status = Some(402);
        diagnostic.provider_id = Some("actual-provider".into());
        diagnostic.model_name = Some("actual-request-model".into());
        diagnostic.provider_code = Some("insufficient_balance".into());
        let error = AgentSendError::from_app_error(AppError::ModelInvocation {
            kind: nomifun_agent_contracts::ModelInvocationFailureKind::QuotaExhausted,
            message: "model emitted a completion report; private provider body".into(),
            diagnostic: diagnostic.clone(),
        });
        assert_eq!(error.code(), Some(AgentErrorCode::UserLlmProviderBillingRequired));
        assert_eq!(error.stream_error().provider_diagnostic, Some(diagnostic));
        assert_eq!(error.stream_error().detail.as_deref(), Some("model emitted a completion report; private provider body"));
        assert_eq!(error.stream_error().retryable, Some(false));
    }

    #[test]
    fn invocation_message_is_visible_redacted_and_cannot_change_typed_classification() {
        use nomifun_agent_contracts::ModelInvocationFailureKind as K;
        let message = "provider returned 401; status delivered; sk-private-value";
        for (kind,reason,expected,stage) in [
            (K::Config,ModelFailureReason::ConfigurationError,AgentErrorCode::UserLlmProviderConfigError,AgentFailureStage::Preparation),
            (K::ProviderError,ModelFailureReason::ProviderUnavailable,AgentErrorCode::UserLlmProviderUnavailable,AgentFailureStage::ModelResponse),
        ] {
            let error = AgentSendError::from_runtime_turn_failure(message,Some(&nomifun_agent_runtime::AgentTurnFailure::ModelInvocation {
                kind,diagnostic:ModelFailureDiagnostic::new(reason),
            }));
            assert_eq!(error.code(),Some(expected));
            assert_eq!(error.stream_error().failure_stage,Some(stage));
            assert_eq!(error.stream_error().retryable,Some(false));
            let detail=error.stream_error().detail.as_ref().unwrap();
            assert!(detail.contains("status delivered")&&!detail.contains("sk-private-value"));
            assert!(error.stream_error().provider_diagnostic.as_ref().unwrap().provider_detail.is_none());
        }
    }

    #[test]
    fn structured_workspace_and_configuration_failures_are_local() {
        let error = AgentSendError::from_app_error(AppError::WorkspaceDirectoryRuntimeUnavailable("/tmp/workspace".into()));
        assert_eq!(error.code(), Some(AgentErrorCode::WorkspaceDirectoryRuntimeUnavailable));
        assert_eq!(error.stream_error().workspace_path.as_deref(), Some("/tmp/workspace"));
        assert_eq!(error.stream_error().failure_stage, Some(AgentFailureStage::Preparation));
        let configuration = AgentSendError::from_app_error(AppError::SessionConfigurationChanged("provider returned 503".into()));
        assert_eq!(configuration.code(), Some(AgentErrorCode::NomifunSessionConfigurationChanged));
        assert_eq!(configuration.ownership(), Some(AgentErrorOwnership::Nomifun));
        assert_eq!(configuration.stream_error().retryable, Some(false));
    }

    #[test]
    fn detail_redaction_still_bounds_and_strips_untrusted_markup() {
        let detail = sanitize_error_detail(&format!("<html>error</html>\nAuthorization: Bearer super-secret\nhttps://example.test/v1?api_key=private-value\n{}", "x".repeat(2000)));
        assert!(!detail.contains("<html>"));
        assert!(!detail.contains("super-secret"));
        assert!(!detail.contains("private-value"));
        assert!(detail.chars().count() <= MAX_DETAIL_CHARS);
    }
}
