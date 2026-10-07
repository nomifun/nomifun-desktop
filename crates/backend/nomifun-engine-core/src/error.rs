use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineProcessError {
    #[error("engine process owner failed: {0}")]
    Process(String),
    #[error("process invocation was cancelled before dispatch")]
    Cancelled,
}

/// Strategy-independent failures at the model-tool/Kernel boundary.
#[derive(Debug, Error)]
pub enum EngineToolError {
    #[error("invalid engine tool contract: {0}")]
    InvalidContract(String),
    #[error("invalid completed model tool call: {0}")]
    InvalidModelEvent(String),
    #[error("tool call arguments exceeded the limit of {limit} bytes")]
    ToolArgumentsTooLarge { limit: usize },
    #[error("tool result exceeded the limit of {limit} bytes")]
    ToolResultTooLarge { limit: usize },
    #[error("tool schema digest mismatch for {tool_name}: expected {expected}, actual {actual}")]
    ToolSchemaDigestMismatch {
        tool_name: String,
        expected: String,
        actual: String,
    },
    #[error("tool plan could not be compiled: {0}")]
    ToolPlan(String),
    #[error("Capability Kernel rejected tool ({code}): {message}")]
    CapabilityKernel { code: String, message: String },
    #[error("tool invocation failed: {0}")]
    ToolInvocation(String),
    #[error("tool invocation was cancelled before dispatch")]
    Cancelled,
}
