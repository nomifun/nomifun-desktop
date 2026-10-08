use nomifun_chat_model_broker::{ChatModelError, ChatModelErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentEngineError {
    #[error("invalid Agent Runtime contract: {0}")]
    InvalidContract(String),
    /// Internal voice-only yield after the original atomic tool admission
    /// rejected a batch with no admitted effects. Never an execution failure
    /// or cancellation of the enclosing Turn.
    #[error("voice correction reached the unadmitted model boundary")]
    VoiceCorrectionBoundary,

    #[error("Agent Runtime turn is already running")]
    TurnAlreadyRunning,

    #[error("Agent Runtime session is disposed")]
    SessionDisposed,

    #[error("Agent Runtime turn {field} does not match the fixed engine binding")]
    TurnBindingMismatch { field: &'static str },

    #[error("Agent Runtime model stream failed ({code:?}): {message}")]
    Model {
        code: ChatModelErrorCode,
        message: String,
        diagnostic: Option<nomifun_agent_contracts::ModelFailureDiagnostic>,
    },

    #[error("Agent Runtime model stream ended without a terminal event")]
    ModelStreamEndedWithoutTerminal,

    #[error("Agent Runtime model emitted an invalid event: {0}")]
    InvalidModelEvent(String),

    #[error("Agent Runtime tool is not exposed by the current plan: {0}")]
    ToolNotExposed(String),

    #[error("Agent Runtime tool call arguments exceeded the limit of {limit} bytes")]
    ToolArgumentsTooLarge { limit: usize },

    #[error("Agent Runtime tool result exceeded the limit of {limit} bytes")]
    ToolResultTooLarge { limit: usize },

    #[error(
        "Agent Runtime tool schema digest mismatch for {tool_name}: expected {expected}, actual {actual}"
    )]
    ToolSchemaDigestMismatch {
        tool_name: String,
        expected: String,
        actual: String,
    },

    #[error("Agent Runtime ToolPlan could not be compiled: {0}")]
    ToolPlan(String),

    #[error("Capability Kernel rejected Agent Runtime Tool ({code}): {message}")]
    CapabilityKernel { code: String, message: String },

    #[error("Nomi workspace context failed: {0}")]
    WorkspaceContext(String),

    #[error("Nomi context assembly failed: {0}")]
    ContextAssembly(String),

    #[error("Nomi context is {actual} bytes, above the {limit} byte limit")]
    ContextTooLarge { limit: usize, actual: usize },

    #[error("Nomi replay contract is invalid: {0}")]
    ReplayContract(String),

    #[error("Nomi compaction failed: {0}")]
    Compaction(String),

    #[error("Nomi compaction failed: compaction ended with MaxOutputTokens")]
    CompactionOutputLimit,

    #[error("Model compaction returned a tool invocation instead of a continuation summary")]
    CompactionInvalidSummary,

    #[error("Nomi process owner failed: {0}")]
    Process(String),

    #[error("Agent Runtime tool invocation failed: {0}")]
    ToolInvocation(String),

    #[error("Agent Runtime event sink failed: {0}")]
    EventSink(String),

    #[error("Agent Runtime turn was cancelled")]
    Cancelled,

    #[error("Agent Runtime turn panicked")]
    TurnPanicked,

    #[error("Agent Runtime turn failed: {0}")]
    TurnFailed(String),
}

impl AgentEngineError {
    pub fn from_model_error(error: ChatModelError) -> Self {
        Self::Model {
            code: error.code,
            message: error.message,
            diagnostic: error.diagnostic,
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_error_preserves_typed_http_status_and_code() {
        use nomifun_agent_contracts::{ModelFailureDiagnostic, ModelFailureReason};
        let mut error = ChatModelError::provider_unavailable("request rejected");
        error.provider_status = Some(503);
        let mut diagnostic = ModelFailureDiagnostic::new(ModelFailureReason::UpstreamServerError);
        diagnostic.http_status = Some(503);
        error.diagnostic = Some(diagnostic.clone());
        assert!(matches!(AgentEngineError::from_model_error(error),
            AgentEngineError::Model { code: ChatModelErrorCode::ProviderUnavailable, message, diagnostic: Some(actual) }
                if message == "request rejected" && actual == diagnostic));
        assert!(matches!(AgentEngineError::from_model_error(ChatModelError::invalid_request("bad input")),
            AgentEngineError::Model { code: ChatModelErrorCode::InvalidRequest, message, diagnostic: None } if message == "bad input"));
    }
}

impl From<nomifun_engine_core::EngineProcessError> for AgentEngineError {
    fn from(error: nomifun_engine_core::EngineProcessError) -> Self {
        match error {
            nomifun_engine_core::EngineProcessError::Process(message) => Self::Process(message),
            nomifun_engine_core::EngineProcessError::Cancelled => Self::Cancelled,
        }
    }
}

impl From<nomifun_engine_core::EngineToolError> for AgentEngineError {
    fn from(error: nomifun_engine_core::EngineToolError) -> Self {
        use nomifun_engine_core::EngineToolError as E;
        match error {
            E::InvalidContract(message) => Self::InvalidContract(message),
            E::InvalidModelEvent(message) => Self::InvalidModelEvent(message),
            E::ToolArgumentsTooLarge { limit } => Self::ToolArgumentsTooLarge { limit },
            E::ToolResultTooLarge { limit } => Self::ToolResultTooLarge { limit },
            E::ToolSchemaDigestMismatch {
                tool_name,
                expected,
                actual,
            } => Self::ToolSchemaDigestMismatch {
                tool_name,
                expected,
                actual,
            },
            E::ToolPlan(message) => Self::ToolPlan(message),
            E::CapabilityKernel { code, message } => Self::CapabilityKernel { code, message },
            E::ToolInvocation(message) => Self::ToolInvocation(message),
            E::Cancelled => Self::Cancelled,
        }
    }
}
