//! Canonical control-input receipts over the existing whole-Execution CAS.
//! The application verifies the original human event and captured target; this
//! boundary persists only its references on the ordinary cancellation event.

use nomifun_api_types::VersionedAgentExecutionCommand;
use nomifun_common::{AgentExecutionActor, AgentExecutionEventKind, AgentExecutionId,
    AgentExecutionStatus, AppError, ConversationId};
use nomifun_db::{IAgentExecutionRepository, NewAgentExecutionEvent};

use crate::{AgentExecutionControlReceipt, AgentExecutionControlSource};

pub(crate) struct CanonicalControlCancellation {
    pub receipt: AgentExecutionControlReceipt,
    pub committed: bool,
}

fn validate_source(owner: &str, actor: &AgentExecutionActor, execution: &str,
    control: &AgentExecutionControlSource) -> Result<(),AppError> {
    AgentExecutionId::try_from(execution).map_err(|_|AppError::BadRequest("canonical control Execution identity is invalid".into()))?;
    ConversationId::try_from(control.agent_session_id.as_str()).map_err(|_|AppError::BadRequest("canonical control Session identity is invalid".into()))?;
    if control.input_event_id.trim().is_empty() || control.input_event_id.len()>1024 {
        return Err(AppError::BadRequest("canonical control input event identity is invalid".into()));
    }
    if !matches!(actor,AgentExecutionActor::User {user_id} if user_id==owner) {
        return Err(AppError::Forbidden("canonical human control requires its authenticated owner".into()));
    }
    Ok(())
}

async fn lookup_cancellation(repository:&dyn IAgentExecutionRepository, owner:&str, execution:&str,
    expected_version:i64, control:&AgentExecutionControlSource)->Result<Option<AgentExecutionControlReceipt>,AppError> {
    let mut after=0;
    let mut found=None;
    loop {
        let events=repository.list_events(owner,execution,after,500).await?;
        for event in &events {
            let payload:serde_json::Value=serde_json::from_str(&event.payload)
                .map_err(|error|AppError::Internal(format!("invalid canonical control Execution event: {error}")))?;
            let Some(source)=payload.get("canonical_control_source") else {continue;};
            let source:AgentExecutionControlSource=serde_json::from_value(source.clone())
                .map_err(|error|AppError::Internal(format!("invalid canonical control source: {error}")))?;
            if &source!=control {continue;}
            if event.event_type!=AgentExecutionEventKind::StatusChanged.as_str()
                || payload.get("status").and_then(serde_json::Value::as_str)!=Some("cancelled")
                || payload.get("reason").and_then(serde_json::Value::as_str)!=Some("cancelled_by_caller")
                || payload.get("canonical_control_expected_version").and_then(serde_json::Value::as_i64)!=Some(expected_version)
                || found.is_some() {
                return Err(AppError::Conflict("canonical control replay differs from its original cancellation target".into()));
            }
            found=Some(AgentExecutionControlReceipt {source,execution_id:execution.into(),
                execution_event_sequence:event.sequence,status:AgentExecutionStatus::Cancelled,replayed:true});
        }
        let Some(last)=events.last() else {break;};
        after=last.sequence;
        if events.len()<500 {break;}
    }
    Ok(found)
}

/// A committed receipt can be acknowledged after source revocation, but a
/// merely accepted control input grants no authority to execute it later.
pub(crate) async fn lookup_receipt(repository:&dyn IAgentExecutionRepository,owner:&str,
    actor:&AgentExecutionActor,execution:&str,command:VersionedAgentExecutionCommand,
    control:&AgentExecutionControlSource)->Result<Option<AgentExecutionControlReceipt>,AppError> {
    validate_source(owner,actor,execution,control)?;
    lookup_cancellation(repository,owner,execution,command.expected_version,control).await
}

pub(crate) async fn cancel_from_source(repository:&dyn IAgentExecutionRepository,owner:&str,
    actor:&AgentExecutionActor,execution:&str,command:VersionedAgentExecutionCommand,
    control:AgentExecutionControlSource)->Result<CanonicalControlCancellation,AppError> {
    validate_source(owner,actor,execution,&control)?;
    if let Some(receipt)=lookup_cancellation(repository,owner,execution,command.expected_version,&control).await? {
        return Ok(CanonicalControlCancellation {receipt,committed:false});
    }
    let before=repository.get_execution_detail(owner,execution).await?
        .ok_or_else(||AppError::NotFound(format!("Agent Execution {execution}")))?;
    let source=crate::execution_source_from_initial_plan(&before.execution.initial_plan_input)?
        .ok_or_else(||AppError::Forbidden("canonical control requires the original exact Execution source".into()))?;
    if source.agent_session_id!=control.agent_session_id
        || before.lead_conversation_id.as_deref()!=Some(control.agent_session_id.as_str()) {
        return Err(AppError::Forbidden("canonical control differs from the Execution's original source Session".into()));
    }
    let status=before.execution.status.parse::<AgentExecutionStatus>()
        .map_err(|error|AppError::Internal(error.to_string()))?;
    if status.is_terminal() {
        // A concurrent winner may have committed after the first lookup.
        if let Some(receipt)=lookup_cancellation(repository,owner,execution,command.expected_version,&control).await? {
            return Ok(CanonicalControlCancellation {receipt,committed:false});
        }
        return Err(AppError::Conflict("a settled execution cannot be cancelled".into()));
    }
    let mut payload=crate::engine::explicit_cancel_payload();
    payload["canonical_control_source"]=serde_json::to_value(&control)
        .map_err(|error|AppError::Internal(error.to_string()))?;
    payload["canonical_control_expected_version"]=serde_json::json!(command.expected_version);
    let cancelled=repository.cancel_execution(owner,execution,command.expected_version,&NewAgentExecutionEvent {
        event_type:AgentExecutionEventKind::StatusChanged,step_id:None,attempt_id:None,
        actor:actor.clone(),payload:payload.to_string(),
    }).await;
    match cancelled {
        Ok(rows)=>Ok(CanonicalControlCancellation {receipt:AgentExecutionControlReceipt {
            source:control,execution_id:execution.into(),execution_event_sequence:rows.execution.event_sequence,
            status:AgentExecutionStatus::Cancelled,replayed:false,
        },committed:true}),
        Err(error)=>{
            // Query only the same source after a CAS/commit race. Never refresh
            // the captured version or issue a second cancellation command.
            if let Some(receipt)=lookup_cancellation(repository,owner,execution,command.expected_version,&control).await? {
                return Ok(CanonicalControlCancellation {receipt,committed:false});
            }
            Err(error.into())
        }
    }
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
