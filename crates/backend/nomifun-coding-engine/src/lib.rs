//! Isolated in-process Coding Agent execution engine.
//!
//! The application composes this engine through model/tool/event ports.
//! Platform owners retain authorization, process ownership and canonical
//! Conversation history; this crate owns execution and derived model context.

#![forbid(unsafe_code)]

mod agents_md;
mod checkpoint;
mod compaction;
mod compaction_source;
mod compacted_history;
pub use compacted_history::CodingCompactedItem;
mod completion;
pub use completion::{CodingCompletionCriterion, CodingCompletionObservation, CodingCompletionReport, CodingCriterionDisposition};
mod context;
mod context_resources;
mod remote_resources;
pub use context_resources::{CodingContextContent, CodingContextResource};
mod context_lifecycle;
mod context_tail;
mod live_context;
pub use live_context::CodingLiveContextPort;
mod media_context;
mod engine;
mod error;
mod events;
mod kernel;
mod history;
mod history_port;
pub use history_port::{CodingHistoryPage, CodingHistoryPort, CodingRecordedTurn};
pub use history::replay_closed_turn;
mod model;
mod output_limit;
mod planning;
mod patch_recovery;
pub use patch_recovery::CodingPatchRecoveryState;
mod requirements;
mod task_continuation;
pub use task_continuation::CodingPriorTask;
pub use requirements::{CodingInputCitation, CodingRequirementOrigin, CodingTaskRequirement};
pub use planning::{CodingPlan, CodingPlanStatus, CodingPlanStep};
mod process;
mod standard_tools;
mod stream_limits;
mod steering;
pub use steering::{CodingInputPort, CodingSteeringInput};
mod tool;
mod tool_dispatch;
mod tool_archive;
mod tool_context;
mod turn;
mod workspace_context;
mod workflow;
pub use workflow::{CodingCommandObservation, CodingWorkStatus};

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
pub use context_lifecycle::CodingModelBudget;
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
