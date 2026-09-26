//! Source-integrated execution loop for the unified Nomi Agent Runtime.
//!
//! The application composes this engine through model/tool/event ports.
//! Platform owners retain authorization, process ownership and canonical
//! Conversation history; this crate owns execution and derived model context.

#![forbid(unsafe_code)]

mod agents_md;
mod adaptive;
mod checkpoint;
mod segments;
pub use segments::{AgentExecutionPressure, AgentExecutionSegmentState, AgentExecutionStopReason, AgentSegmentPolicy, AgentSegmentReason};
mod recovery;
mod reconciliation;
pub use reconciliation::{AgentReconciledOutcome, AgentReconciledResume, AgentReconciliationSource, reconcile_execution_tail};
pub use recovery::AgentTurnRecovery;
pub use checkpoint::{AgentCheckpointReceipt, AgentExecutionCheckpoint};
pub use adaptive::{AgentRuntimeActivationReason, AgentRuntimeModule};
mod compaction;
mod compaction_source;
mod compacted_history;
pub use compacted_history::AgentCompactedItem;
mod completion;
pub use completion::{AgentCompletionCriterion, AgentCompletionObservation, AgentCompletionReport, AgentCriterionDisposition};
mod context;
mod context_resources;
mod remote_resources;
pub use context_resources::{AgentContextContent, AgentContextResource};
mod context_lifecycle;
mod context_tail;
mod live_context;
pub use live_context::AgentLiveContextPort;
mod media_context;
mod engine;
mod error;
mod events;
mod execution_policy;
mod kernel;
mod history;
mod history_port;
pub use history_port::{AgentHistoryPage, AgentHistoryPort, AgentRecordedTurn};
pub use history::replay_closed_turn;
mod model;
mod output_limit;
mod public_output;
mod protocol_recovery;
mod planning;
mod patch_recovery;
pub use patch_recovery::AgentPatchRecoveryState;
mod requirements;
mod task_continuation;
pub use task_continuation::AgentPriorTask;
pub use requirements::{AgentInputCitation, AgentRequirementOrigin, AgentTaskRequirement};
pub use planning::{AgentPlan, AgentPlanStatus, AgentPlanStep};
mod standard_tools;
mod stream_limits;
mod steering;
pub use steering::{AgentInputPort, AgentSteeringInput};
mod tool;
mod tool_dispatch;
mod tool_validation;
mod tool_discovery;
mod tool_archive;
mod tool_context;
mod turn;
mod workspace_context;
mod workflow;
pub use workflow::{AgentCommandObservation, AgentWorkStatus};

pub use engine::{
    AgentEngine, AgentEngineBuild, AgentEngineSession, EngineBinding, EngineBuildId,
};
pub use agents_md::{
    load_agents_md, AgentsMdContext, AgentsMdLayer, AgentsMdPolicy, AgentWorkspaceReader,
};
pub use compaction::{
    run_compaction, AgentCompactionRequest, AgentCompactionSummary,
};
pub use context::{
    AgentContextAssembler, AgentContextBudget, AgentContextDiagnostics,
};
pub use context_lifecycle::AgentModelBudget;
pub use error::AgentEngineError;
pub use events::{AgentEngineEvent, AgentEventSink, NoopAgentEventSink};
pub use kernel::{
    compile_agent_tool_plan, AgentToolExposure, KernelAgentToolInvoker,
};
pub use model::{BrokerAgentModelPort, AgentModelPort, AgentModelStream};
pub use standard_tools::standard_agent_tool_exposures;
pub use tool::{
    input_schema_digest, AgentEffectClass, AgentToolBinding, AgentToolInvocation,
    AgentToolInvoker, AgentToolPlan, AgentToolResult,
};
pub use tool_discovery::{
    AgentToolDiscoveryCandidate, AgentToolDiscoveryPort,
    MAX_MATCHES as MAX_TOOL_DISCOVERY_MATCHES,
};
pub use turn::{AgentControlRejectionState, AgentTurnRequest, AgentTurnResult, AgentTurnTerminal};
