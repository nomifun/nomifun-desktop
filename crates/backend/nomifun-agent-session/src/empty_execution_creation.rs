//! Retire only an untouched Session created for this original Attempt. Empty
//! proof and deletion admission share the input/link writer, never a preflight.
use super::*;

#[derive(Debug,Clone)]
pub enum EmptyExecutionCreationFence {
    Missing,
    NotEmpty {session_id:AgentSessionId},
    AwaitingAttemptDisposition {session_id:AgentSessionId},
    Fenced {command:DeleteAgentSessionCommand,session:AgentSessionLiveRecord},
}

impl AgentSessionStore {
    pub async fn fence_empty_execution_creation(&self,owner:&PrincipalRef,attempt:&str,
        expected_session:Option<&AgentSessionId>,requested_at:i64)->Result<EmptyExecutionCreationFence,SessionStoreError> {
        validate_principal(owner)?;validate_uuidv7(attempt,"original Attempt")?;
        if owner.principal_kind!="user"||requested_at<0 {return Err(SessionStoreError::ExecutionFenced);}
        let normal=format!("user:{}:open:{attempt}",owner.principal_id);
        let directed=format!("user:{}:open:execution-source:{attempt}",owner.principal_id);
        let (mut tx,guard)=self.begin_write_transaction().await?;
        let openings:Vec<StoredEventRow>=sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE producer_id='session_api' AND idempotency_key IN(?,?) ORDER BY session_id LIMIT 3")
            .bind(&normal).bind(&directed).fetch_all(&mut *tx).await?;
        if openings.is_empty() {guard.commit(tx).await?;return Ok(EmptyExecutionCreationFence::Missing);}
        let [opening]=openings.as_slice() else {return Err(SessionStoreError::Conflict("Attempt creation has ambiguous original openings".into()));};
        let session_id=AgentSessionId::from(opening.session_id.as_str());
        let row=session_row_by_id_tx(&mut tx,session_id.as_ref()).await?;require_owner(&row,owner)?;
        if expected_session.is_some_and(|expected|expected!=&session_id) {
            guard.commit(tx).await?;return Ok(EmptyExecutionCreationFence::NotEmpty {session_id});
        }
        let session=live_from_row(row)?;
        let source=nomifun_db::original_attempt_creation_source_tx(&mut tx,&owner.principal_id,attempt).await
            .map_err(native_resume_queue::admission_error)?;
        let key=if source.is_some() {&directed} else {&normal};
        let opening_id=format!("opening:{key}");
        let correlation=format!("open:{key}");
        let original=event_from_row(opening.clone())?;
        let opening_payload=payload_value_for_event_tx(&mut tx,&original).await?;
        let initial=opening.seq==1&&opening.kind=="session/opening"&&opening.kind_version==1
            &&opening.idempotency_key==*key&&opening.event_id==opening_id&&opening.correlation_id==correlation
            &&opening.causation_event_id.is_none()&&opening_payload.get("initial_input").is_none()
            &&opening_payload.get("agent_binding")==Some(&serde_json::to_value(&session.agent_binding)?);
        let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=?) OR EXISTS(SELECT 1 FROM agent_turns WHERE session_id=?) OR EXISTS(SELECT 1 FROM agent_effects WHERE session_id=?)")
            .bind(session_id.as_ref()).bind(session_id.as_ref()).bind(session_id.as_ref()).fetch_one(&mut *tx).await?;
        let events:Vec<StoredEventRow>=sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? ORDER BY seq LIMIT 4")
            .bind(session_id.as_ref()).fetch_all(&mut *tx).await?;
        let head=head_by_id_tx(&mut tx,session_id.as_ref()).await?;
        let mut untouched=initial&&!occupied&&matches!(events.len(),2|3)&&head.active_turn_id.is_none()
            &&head.active_set_generation==0&&head.last_seq==events.len() as u64&&session.next_seq==events.len() as u64+1;
        for event in events.iter().skip(1) {
            let record=event_from_row(event.clone())?;let payload=payload_value_for_event_tx(&mut tx,&record).await?;
            untouched&=event.kind_version==1&&event.causation_event_id.as_deref()==Some(opening_id.as_str());
            untouched&=match event.seq {
                2=>event.kind=="capability/active-set-committed"&&event.producer_id=="session_api:capability-host"
                    &&event.event_id==format!("active-set:{key}")&&event.idempotency_key==format!("{key}:active-set-0")
                    &&event.correlation_id==correlation&&payload["generation"]==0&&payload["delta"]==json!([]),
                3=>event.kind=="session/ready"&&event.producer_id=="runtime_supervisor"
                    &&event.event_id==format!("ready:{key}")&&event.idempotency_key==format!("{key}:ready")
                    &&event.correlation_id==format!("session:{}",session_id.as_ref())
                    &&payload["resolved_snapshot_ref"]==serde_json::to_value(&session.agent_binding.resolved_snapshot_ref)?,
                _=>false,
            };
        }
        untouched&=if events.len()==2 {head.status=="opening"}else{head.status=="ready"};
        if !untouched {guard.commit(tx).await?;return Ok(EmptyExecutionCreationFence::NotEmpty {session_id});}
        // Losing a lease is not abandonment of the original reservation: a
        // successor can still own this same Queued Attempt and its opening.
        // Wait for the domain's durable disposition, never a timeout or an
        // empty canonical transcript, before admitting retirement.
        let abandoned:bool=sqlx::query_scalar("SELECT attempt.status IN('completed','failed','cancelled','interrupted') OR step.status IN('completed','failed','cancelled','skipped') OR step.superseded_in_revision IS NOT NULL OR execution.status IN('completed','completed_with_failures','failed','cancelled') OR execution.deleted_at IS NOT NULL FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id JOIN agent_executions execution ON execution.execution_id=attempt.execution_id WHERE attempt.attempt_id=? AND execution.user_id=?")
            .bind(attempt).bind(&owner.principal_id).fetch_one(&mut *tx).await?;
        if !abandoned {guard.commit(tx).await?;return Ok(EmptyExecutionCreationFence::AwaitingAttemptDisposition {session_id});}
        let command=DeleteAgentSessionCommand {agent_session_id:session_id.clone(),owner_ref:owner.clone(),requested_at,
            operation_id:format!("delete:user:{}:{}:discard-empty-execution:{attempt}",owner.principal_id,session_id.as_ref()).into()};
        let changed=sqlx::query("UPDATE agent_sessions SET state='deleting' WHERE agent_session_id=? AND state IN('live','deleting')")
            .bind(session_id.as_ref()).execute(&mut *tx).await?;
        if changed.rows_affected()!=1 {return Err(SessionStoreError::ExecutionFenced);}
        guard.commit(tx).await?;
        Ok(EmptyExecutionCreationFence::Fenced {command,session})
    }
}
