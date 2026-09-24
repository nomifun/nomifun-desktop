use nomifun_chat_model_broker::{ChatModelError, ChatModelErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentEngineError {
    #[error("invalid Agent Runtime contract: {0}")]
    InvalidContract(String),

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
        // Preserve the Broker's trusted HTTP status through the existing
        // string boundary. Only this numeric metadata is added; redaction of the
        // existing message remains the Broker owner's responsibility.
        let message = match error.provider_status {
            Some(status) => format!("provider_http_status={status}; {}", error.message),
            None => error.message,
        };
        Self::Model {
            code: error.code,
            message,
        }
    }

    pub fn model_stream_interrupted(message: impl Into<String>) -> Self {
        let error = ChatModelError::new(
            ChatModelErrorCode::StreamInterrupted,
            message,
            nomifun_chat_model_broker::ChatRetryDirective::Never,
        );
        Self::from_model_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_error_preserves_typed_http_status_and_code() {
        let mut error = ChatModelError::provider_unavailable("request rejected");
        error.provider_status = Some(503);
        assert!(matches!(AgentEngineError::from_model_error(error),
            AgentEngineError::Model { code: ChatModelErrorCode::ProviderUnavailable, message }
                if message == "provider_http_status=503; request rejected"));
        assert!(matches!(AgentEngineError::from_model_error(ChatModelError::invalid_request("bad input")),
            AgentEngineError::Model { code: ChatModelErrorCode::InvalidRequest, message } if message == "bad input"));
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
