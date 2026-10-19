//! Read-only activity projection. These rows are navigation/control targets
//! derived from the original owners; they are not another execution ledger.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentActivityTarget {
    SessionInput {
        agent_session_id: String,
        message_id: String,
    },
    SessionTurn {
        agent_session_id: String,
        operation_id: String,
    },
    ExecutionTask {
        execution_id: String,
        step_id: String,
        attempt_id: String,
        expected_execution_version: i64,
        expected_step_version: i64,
    },
    ExecutionStep {
        execution_id: String,
        step_id: String,
        expected_execution_version: i64,
        expected_step_version: i64,
    },
    ExecutionGroup {
        execution_id: String,
        expected_version: i64,
    },
    CronRun {
        cron_job_id: String,
        cron_job_run_id: String,
    },
    AutoWorkRun {
        target_kind: crate::AutoWorkTargetKind,
        target_id: String,
        requirement_id: String,
        claim_generation: i64,
    },
    RemoteSession {
        agent_session_id: String,
        operation_id: Option<String>,
        expected_head_seq: u64,
        expected_binding_version: i64,
    },
    RuntimeCarrier {
        agent_session_id: String,
        instance_id: String,
        expected_generation: Option<u64>,
        expected_head_seq: u64,
    },
    BrowserWorkspace {
        agent_session_id: String,
        service_instance_id:String,
        runtime_generation: u64,
        expected_head_seq: u64,
    },
    SessionResources {
        agent_session_id: String,
        expected_head_seq: u64,
    },
    KnowledgeWriteback {
        agent_session_id: String,
        operation_id: String,
    },
    ProcessInstance {
        agent_session_id: String,
        owner_generation: String,
        process_id: String,
        creator_turn_id: String,
        creator_call_id: String,
    },
    SshLink {
        agent_session_id: String,
        ssh_host_id: String,
        pool_instance_id: String,
        instance_id: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityKind {
    Conversation,
    Companion,
    DelegatedTask,
    CronRun,
    AutoWorkRun,
    RemoteSession,
    Resources,
    KnowledgeWriteback,
    Collaboration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityState {
    Queued,
    Preparing,
    Planning,
    Running,
    Waiting,
    Paused,
    Recovering,
    Stopping,
    Completed,
    Failed,
    Cancelled,
    Unconfirmed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityContinuation {
    None,
    AnswerQueued,
    ResumeQueued,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityResourceKind {
    Runtime,
    Processes,
    Browser,
    Ssh,
    KnowledgeWriteback,
    Effects,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityResourceState {
    Active,
    HeldForUser,
    Releasing,
    Released,
    Unconfirmed,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentActivityResource {
    pub kind: AgentActivityResourceKind,
    pub state: AgentActivityResourceState,
    /// Domain-owned, bounded explanation. No credentials or model input.
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<AgentActivityTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentActivity {
    /// Stable display key only; commands use the exact typed target below.
    pub key: String,
    pub title: String,
    pub kind: AgentActivityKind,
    pub state: AgentActivityState,
    pub agent_session_id: Option<String>,
    pub target: AgentActivityTarget,
    /// A true AutoWork root keeps its Requirement run owner as the primary
    /// target. This exact Step target permits only its delegated-work scope.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub delegation_target:Option<AgentActivityTarget>,
    pub continuation: AgentActivityContinuation,
    pub waiting_reason: Option<String>,
    pub stop_accepted: bool,
    pub execution_stopped: bool,
    pub resources: Vec<AgentActivityResource>,
    pub can_stop: bool,
    pub can_retry_cleanup: bool,
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentActivitiesResponse {
    pub items: Vec<AgentActivity>,
    pub next_cursor: Option<String>,
}

/// A finite read snapshot. Retrying individual commands never captures a
/// successor task; this value is not a second persistent cancellation ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentActivityStopSnapshot {
    pub captured_at_ms: i64,
    pub targets: Vec<AgentActivityTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopAgentActivityRequest {
    pub target: AgentActivityTarget,
    pub idempotency_key: String,
    #[serde(default)]
    pub retry_cleanup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<nomifun_common::AgentExecutionCancellationScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentActivityCommandReceipt {
    pub accepted: bool,
    pub execution_stopped: bool,
    pub resources: Vec<AgentActivityResource>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_stop_targets_require_exact_owner_identity_and_keep_scope_explicit() {
        let request = serde_json::json!({"target":{"kind":"remote_session","agent_session_id":"session","operation_id":"original-operation","expected_head_seq":17,"expected_binding_version":3},
            "idempotency_key":"original-stop"});
        assert!(serde_json::from_value::<StopAgentActivityRequest>(request.clone()).is_ok());
        for field in [
            "operation_id",
            "expected_head_seq",
            "expected_binding_version",
        ] {
            let mut missing = request.clone();
            missing["target"].as_object_mut().unwrap().remove(field);
            // None is explicit for a genuine opening with no Turn. Serde's
            // Option may omit it, but the writer still compares None exactly.
            if field != "operation_id" {
                assert!(serde_json::from_value::<StopAgentActivityRequest>(missing).is_err());
            }
        }
        let process = serde_json::json!({"kind":"process_instance","agent_session_id":"session","owner_generation":"original-owner","process_id":"original-handle","creator_turn_id":"original-turn","creator_call_id":"original-call"});
        assert!(serde_json::from_value::<AgentActivityTarget>(process.clone()).is_ok());
        let mut missing = process;
        missing.as_object_mut().unwrap().remove("creator_call_id");
        assert!(serde_json::from_value::<AgentActivityTarget>(missing).is_err());
        let resource = serde_json::json!({"kind":"ssh_link","agent_session_id":"session","ssh_host_id":"host","pool_instance_id":"original-pool","instance_id":11,"force":true});
        assert!(serde_json::from_value::<AgentActivityTarget>(resource).is_err());
    }
}
