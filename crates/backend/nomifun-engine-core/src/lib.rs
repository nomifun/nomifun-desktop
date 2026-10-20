//! Ports shared by source-integrated execution engines. No strategy, Session
//! database, plugin loader or ambient authority is provided by this crate.
//! The application supplies exact compiled capabilities and owns admitted
//! effects through cleanup; engines choose their own loops and context policy.

#![forbid(unsafe_code)]

mod error;
mod context_resource;
mod kernel;
mod process;
mod system_process;
mod system_permission;
mod tool;
mod tool_display;
mod tool_file_display;
pub use tool_file_display::ToolFileProjection;
mod tool_schema;
pub use tool_schema::model_tool_schema;
pub use tool_display::{ToolDisplayFacts, ToolBoundedResult, WrittenKnowledgeFile, tool_display_target, tool_result_target, tool_result_display, tool_display_operation, tool_denied_stop_target};

pub use error::{EngineProcessError, EngineToolError};
pub use system_permission::{SystemPermissionKind, SystemPermissionRequired, SYSTEM_PERMISSION_REQUIRED,
    project_system_permission_required, system_permission_error_carrier};
pub use context_resource::{EngineContextContent, EngineContextResource, EngineResourceQuery, EngineResourceRead, EngineResourceImageRead, EngineResourcePort};
pub use context_resource::mcp_template_variables_schema;
pub use kernel::{EngineToolExposure, KernelEngineToolInvoker, compile_engine_tool_plan};
pub use nomifun_chat_model_broker::{BrokerEngineModelPort, EngineModelPort, EngineModelStream};
pub use nomi_process_runtime::MAX_PTY_DIMENSION;
pub use process::{
    EngineCleanupReport, EngineExecutionIdentity, EngineProcessOutput, EngineProcessPoll, EngineProcessRequest,
    EngineProcessSession, EngineProcessStartError, EngineProcessTransport, ManagedEngineProcessOwner, EngineProcessCancellation, EngineProcessCancellationReceipt, EngineProcessOutputReplay,
    EngineProcessTerminalMetadata,
};
pub use tool::{
    EngineEffectClass, EngineToolBinding, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult, RuntimeToolHoldReason, input_schema_digest, parse_completed_arguments, validate_tool_argument_size,
    tool_text_excerpt_ranges,
};
