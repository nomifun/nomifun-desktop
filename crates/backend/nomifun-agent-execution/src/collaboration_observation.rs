//! Disposable, read-only observations for native parent coordination. Launch
//! snapshots and participant configuration are outside this query surface.
use crate::{AgentExecutionEngine, domain_mapper};
use nomifun_api_types::AgentExecutionEvent;
use nomifun_common::{AgentExecutionStatus, AppError, ExecutionAttemptStatus, ExecutionStepKind, ExecutionStepStatus};
use nomifun_db::AgentExecutionObservationHeader;

#[derive(Debug, Clone)]
pub struct AgentExecutionCollaborationObservation {
    pub execution: AgentExecutionObservationHeader,
    pub state: Option<AgentExecutionCollaborationState>,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionCollaborationState {
    pub steps: Vec<AgentExecutionCollaborationStep>,
    pub attempts: Vec<AgentExecutionCollaborationAttempt>,
    pub events: Vec<AgentExecutionEvent>,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionCollaborationStep {
    pub step_id: String,
    pub title: String,
    pub kind: ExecutionStepKind,
    pub status: ExecutionStepStatus,
    pub version: i64,
    pub superseded_in_revision: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionCollaborationAttempt {
    pub attempt_id: String,
    pub step_id: String,
    pub attempt_no: i64,
    pub status: ExecutionAttemptStatus,
    pub question: Option<String>,
    pub error: Option<String>,
    pub output_summary: Option<String>,
    pub runtime_state: Option<serde_json::Value>,
    pub conversation_id: Option<String>,
    pub turn_operation_key: String,
}

fn invalid(error: impl std::fmt::Display) -> AppError { AppError::Internal(format!("invalid collaboration observation: {error}")) }

impl AgentExecutionEngine {
    /// Read one owner-scoped snapshot, incrementally consuming immutable domain
    /// events. This method never starts or recovers an Execution.
    pub async fn collaboration_observation(&self, owner: &str, execution: &str, after: Option<i64>)
        -> Result<AgentExecutionCollaborationObservation, AppError> {
        let rows = self.repository.collaboration_observation(owner,execution,after).await?
            .ok_or_else(||AppError::NotFound("selected Execution is unavailable".into()))?;
        nomifun_common::validate_uuidv7(&rows.execution.execution_id).map_err(invalid)?;
        rows.execution.status.parse::<AgentExecutionStatus>()
            .map_err(|error|AppError::Internal(format!("invalid collaboration Execution status: {error}")))?;
        let state=rows.state.map(|state| {
            let steps=state.steps.into_iter().map(|step| {
                nomifun_common::validate_uuidv7(&step.step_id).map_err(invalid)?;
                Ok::<_,AppError>(AgentExecutionCollaborationStep {step_id:step.step_id,title:step.title,
                    kind:step.kind.parse().map_err(invalid)?,status:step.status.parse().map_err(invalid)?,version:step.version,
                    superseded_in_revision:step.superseded_in_revision})
            }).collect::<Result<_,_>>()?;
            let attempts=state.attempts.into_iter().map(|attempt| {
                nomifun_common::validate_uuidv7(&attempt.attempt_id).map_err(invalid)?;
                nomifun_common::validate_uuidv7(&attempt.step_id).map_err(invalid)?;
                if let Some(session)=attempt.conversation_id.as_deref() {nomifun_common::validate_uuidv7(session).map_err(invalid)?;}
                let effects=nomifun_db::AttemptConversationEffects::decode(attempt.runtime_state.as_deref()).map_err(invalid)?;
                let turn_operation_key=nomifun_db::execution_attempt_turn_key_from_facts(&attempt.attempt_id,&effects,attempt.last_turn_operation_key)?;
                Ok::<_,AppError>(AgentExecutionCollaborationAttempt {attempt_id:attempt.attempt_id,step_id:attempt.step_id,
                    attempt_no:attempt.attempt_no,status:attempt.status.parse().map_err(invalid)?,question:attempt.question,
                    error:attempt.error,output_summary:attempt.output_summary,
                    runtime_state:attempt.runtime_state.map(|value|serde_json::from_str(&value).map_err(invalid)).transpose()?,
                    conversation_id:attempt.conversation_id,turn_operation_key})
            }).collect::<Result<_,_>>()?;
            Ok::<_,AppError>(AgentExecutionCollaborationState {steps,attempts,
                events:state.events.into_iter().map(domain_mapper::event).collect::<Result<_,_>>()?})
        }).transpose()?;
        Ok(AgentExecutionCollaborationObservation {execution:rows.execution,state})
    }

    /// Read only domain and native cursors when a wake hint may have been lost.
    pub async fn collaboration_revisions(&self, owner: &str, executions: &[String], sessions: &[String])
        -> Result<nomifun_db::AgentCollaborationObservationRevisions, AppError> {
        self.repository.collaboration_revisions(owner,executions,sessions).await.map_err(Into::into)
    }
}
