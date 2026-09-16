//! Ports shared by source-integrated execution engines. No strategy, Session
//! database, plugin loader or ambient authority is provided by this crate.
//! The application supplies exact compiled capabilities and owns admitted
//! effects through cleanup; engines choose their own loops and context policy.

#![forbid(unsafe_code)]

mod error;
mod context_resource;
mod kernel;
mod process;
mod tool;

pub use error::{EngineProcessError, EngineToolError};
pub use context_resource::{EngineContextContent, EngineContextResource, EngineResourceQuery, EngineResourceRead, EngineResourceImageRead, EngineResourcePort};
pub use context_resource::mcp_template_variables_schema;
pub use kernel::{EngineToolExposure, KernelEngineToolInvoker, compile_engine_tool_plan};
pub use nomifun_chat_model_broker::{BrokerEngineModelPort, EngineModelPort, EngineModelStream};
pub use process::{
    EngineCleanupReport, EngineProcessOutput, EngineProcessPoll, EngineProcessRequest,
    EngineProcessSession, EngineProcessStartError, EngineProcessTransport, ManagedEngineProcessOwner,
};
pub use tool::{
    EngineEffectClass, EngineToolBinding, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult, input_schema_digest, parse_completed_arguments, validate_tool_argument_size,
};
