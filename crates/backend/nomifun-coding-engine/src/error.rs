use nomifun_chat_model_broker::{ChatModelError, ChatModelErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodingEngineError {
    #[error("invalid coding engine contract: {0}")]
    InvalidContract(String),

    #[error("engine build was not found: {0}")]
    EngineBuildNotFound(String),

    #[error(
        "engine build digest mismatch for {engine_build_id}: expected {expected}, actual {actual}"
    )]
    EngineBuildDigestMismatch {
        engine_build_id: String,
        expected: String,
        actual: String,
    },

    #[error("unsupported coding runtime profile: {0}")]
    UnsupportedProfile(String),

    #[error("coding turn is already running")]
    TurnAlreadyRunning,

    #[error("coding engine session is disposed")]
    SessionDisposed,

    #[error("coding turn {field} does not match the fixed engine binding")]
    TurnBindingMismatch { field: &'static str },

    #[error("coding turn has no active operation")]
    NoActiveTurn,

    #[error("coding turn exceeded the model-step limit of {0}")]
    ModelStepLimitExceeded(u16),

    #[error("coding model stream failed ({code:?}): {message}")]
    Model {
        code: ChatModelErrorCode,
        message: String,
    },

    #[error("coding model stream ended without a terminal event")]
    ModelStreamEndedWithoutTerminal,

    #[error("coding model emitted an invalid event: {0}")]
    InvalidModelEvent(String),

    #[error("coding tool is not exposed by the current plan: {0}")]
    ToolNotExposed(String),

    #[error("coding tool call arguments exceeded the limit of {limit} bytes")]
    ToolArgumentsTooLarge { limit: usize },

    #[error("coding tool result exceeded the limit of {limit} bytes")]
    ToolResultTooLarge { limit: usize },

    #[error(
        "coding tool schema digest mismatch for {tool_name}: expected {expected}, actual {actual}"
    )]
    ToolSchemaDigestMismatch {
        tool_name: String,
        expected: String,
        actual: String,
    },

    #[error("coding ToolPlan could not be compiled: {0}")]
    ToolPlan(String),

    #[error("Capability Kernel rejected Coding Tool ({code}): {message}")]
    CapabilityKernel { code: String, message: String },

    #[error("Coding workspace context failed: {0}")]
    WorkspaceContext(String),

    #[error("Coding context assembly failed: {0}")]
    ContextAssembly(String),

    #[error("Coding context is {actual} bytes, above the {limit} byte limit")]
    ContextTooLarge { limit: usize, actual: usize },

    #[error("Coding checkpoint is invalid: {0}")]
    Checkpoint(String),

    #[error("Coding compaction failed: {0}")]
    Compaction(String),

    #[error("Coding process owner failed: {0}")]
    Process(String),

    #[error("coding tool invocation failed: {0}")]
    ToolInvocation(String),

    #[error("coding event sink failed: {0}")]
    EventSink(String),

    #[error("coding turn was cancelled")]
    Cancelled,

    #[error("coding turn panicked")]
    TurnPanicked,

    #[error("coding turn failed: {0}")]
    TurnFailed(String),
}

impl CodingEngineError {
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
        assert!(matches!(CodingEngineError::from_model_error(error),
            CodingEngineError::Model { code: ChatModelErrorCode::ProviderUnavailable, message }
                if message == "provider_http_status=503; request rejected"));
        assert!(matches!(CodingEngineError::from_model_error(ChatModelError::invalid_request("bad input")),
            CodingEngineError::Model { code: ChatModelErrorCode::InvalidRequest, message } if message == "bad input"));
    }
}

impl From<nomifun_engine_core::EngineProcessError> for CodingEngineError {
    fn from(error: nomifun_engine_core::EngineProcessError) -> Self {
        match error {
            nomifun_engine_core::EngineProcessError::Process(message) => Self::Process(message),
            nomifun_engine_core::EngineProcessError::Cancelled => Self::Cancelled,
        }
    }
}

impl From<nomifun_engine_core::EngineToolError> for CodingEngineError {
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
