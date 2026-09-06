//! Isolated in-process Coding Agent execution engine.
//!
//! This crate is intentionally not wired into the current application
//! composition root. It provides the second execution engine behind narrow
//! NomiFun ports so it can be developed and validated independently before
//! remote-main integration.

#![forbid(unsafe_code)]

mod agents_md;
mod checkpoint;
mod compaction;
mod context;
mod engine;
mod error;
mod events;
mod kernel;
mod model;
mod process;
mod standard_tools;
mod tool;
mod turn;

pub use engine::{
    AgentEngine, CodingEngine, CodingEngineBuild, CodingEngineCatalog, CodingEngineChannel,
    CodingEngineSelector, CodingEngineSession, CodingRuntimeProfile, EngineBinding,
    EngineBuildId, EngineFamilyId,
};
pub use agents_md::{
    load_agents_md, AgentsMdContext, AgentsMdLayer, AgentsMdPolicy, CodingWorkspaceReader,
};
pub use checkpoint::{CheckpointAdmission, CheckpointDiscardReason, CodingCheckpoint};
pub use compaction::{
    run_compaction, CodingCompactionRequest, CodingCompactionSummary,
};
pub use context::{
    CodingContextAssembler, CodingContextBudget, CodingContextDiagnostics,
};
pub use error::CodingEngineError;
pub use events::{CodingEngineEvent, CodingEventSink, NoopCodingEventSink};
pub use kernel::{
    compile_coding_tool_plan, CodingToolExposure, KernelCodingToolInvoker,
};
pub use model::{BrokerCodingModelPort, CodingModelPort, CodingModelStream};
pub use process::{
    CodingCleanupReport, CodingProcessOutput, CodingProcessPoll, CodingProcessRequest,
    CodingProcessSession, CodingProcessTransport, ManagedCodingProcessOwner,
};
pub use standard_tools::{
    standard_coding_tool_exposures, StandardCodingToolLevel,
};
pub use tool::{
    input_schema_digest, CodingEffectClass, CodingToolBinding, CodingToolInvocation,
    CodingToolInvoker, CodingToolPlan, CodingToolResult,
};
pub use turn::{CodingTurnRequest, CodingTurnResult, CodingTurnTerminal};
