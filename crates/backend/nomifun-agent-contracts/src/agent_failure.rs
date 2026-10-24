//! Failure provenance selected by the owner at the failing boundary.
//! Diagnostic text is never a classifier, retry permission or task receipt.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFailureStage {
    /// The host knows the original turn boundary but has no finer stage.
    TurnExecution,
    Preparation,
    ContextAssembly,
    ModelResponse,
    ToolValidation,
    ToolExecution,
    CompletionReport,
    Persistence,
    ResourceCleanup,
    Recovery,
}

/// Why this Turn cannot finish the accepted work. Completed effects and
/// independently completed child tasks keep their original results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskIncompleteReason {
    BlockedWork,
    StepLimit,
    OutputTruncated,
    UnresolvedPlan,
    UnverifiedChanges,
    RunningProcesses,
    UnverifiedCompletion,
    RejectedControl,
    NoProgress,
    ExecutionGuard,
    RecoveryGuard,
    CompletionReportRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResourceBindingFailure {
    pub missing_resource_kinds: Vec<String>,
    pub missing_resource_operations: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
}
