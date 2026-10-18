//! The real Session process owner's release receipt. No table, PID probing,
//! reconstructed process, checkpoint conversion, or second ownership ledger.
use super::*;
use nomifun_agent_contracts::{NativeRetainedProcess, NativeSessionProcessRetention, SessionProcessOwnerReleasedV1, SESSION_PROCESS_OWNER_RELEASED};

fn release_key(session:&AgentSessionId,generation:&str) -> String { format!("process-owner-release:{}:{generation}",session.as_ref()) }

async fn retained_session_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId) ->Result<AgentSessionLiveRecord,SessionStoreError> {
    live_from_row(session_row_by_id_tx(tx,session.as_ref()).await?)
}

pub(super) async fn validate_retention_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,retention:&NativeSessionProcessRetention)
    ->Result<(),SessionStoreError> {
    retention.validate().map_err(SessionStoreError::InvalidPayload)?;
    let live=retained_session_tx(tx,&event.agent_session_id).await?;
    if event.kind.0!="runtime/progress-recorded" || event.producer_id.as_ref()!="runtime_supervisor"
        || retention.agent_session_id!=event.agent_session_id || retention.owner_ref!=live.owner_ref {return Err(SessionStoreError::ExecutionFenced);}
    if event_by_event_id_tx(tx,&release_key(&event.agent_session_id,&retention.owner_generation)).await?
        .is_some_and(|released| released.seq<event.seq as i64) {return Err(SessionStoreError::ExecutionFenced);}
    for process in &retention.processes {
        let started:Option<String>=sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(event.agent_session_id.as_ref()).bind(process.creator_turn_id.as_ref()).fetch_optional(&mut **tx).await?;
        let started=started.ok_or_else(||SessionStoreError::InvalidPayload("retained process creator Turn is missing".into()))?;
        let source=event_by_event_id_tx(tx,&started).await?.map(event_from_row).transpose()?
            .ok_or_else(||SessionStoreError::InvalidPayload("retained process creator start is missing".into()))?;
        if source.seq>=event.seq || payload_value_for_event_tx(tx,&source).await?.get("resolved_snapshot_ref")!=Some(&serde_json::to_value(&process.resolved_snapshot_ref)?) {
            return Err(SessionStoreError::InvalidPayload("retained process differs from its frozen creator Snapshot".into()));
        }
    }
    Ok(())
}

pub(super) async fn retained_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,generation:&str,through:u64)
    -> Result<(BTreeMap<String,NativeRetainedProcess>,Option<EventId>),SessionStoreError> {
    let live=retained_session_tx(tx,session).await?;
    let mut processes=BTreeMap::new();let mut last=None;
    let mut cursor=0i64;
    loop {
    let rows:Vec<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
        FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND e.kind='runtime/progress-recorded' AND e.seq>? AND e.seq<? \
        AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='host_cleanup_proven' \
        AND COALESCE(json_extract(e.inline_json,'$.event.session_process_retention.owner_generation'),json_extract(CAST(p.body AS TEXT),'$.value.event.session_process_retention.owner_generation'))=? ORDER BY e.seq LIMIT 16")
        .bind(session.as_ref()).bind(cursor).bind(as_i64(through,"process retention cursor")?).bind(generation).fetch_all(&mut **tx).await?;
    if rows.is_empty() {break;}
    for row in rows {
        cursor=cursor.max(row.seq);
        let event=event_from_row(row)?;let value=payload_value_for_event_tx(tx,&event).await?;
        if value.pointer("/event/event").and_then(Value::as_str)!=Some("host_cleanup_proven") {continue;}
        let Some(body)=value.pointer("/event/session_process_retention") else {continue;};
        let retained:NativeSessionProcessRetention=serde_json::from_value(body.clone())?;
        validate_retention_tx(tx,&event,&retained).await?;
        if retained.agent_session_id!=*session || retained.owner_ref!=live.owner_ref || event.producer_id.as_ref()!="runtime_supervisor" {
            return Err(SessionStoreError::ExecutionFenced);
        }
        if retained.owner_generation!=generation {continue;}
        if processes.len()+retained.processes.iter().filter(|process|!processes.contains_key(&process.process_id)).count()>64 {
            let reaped=super::session_process_retirement::confirmed_tx(tx,session,&processes,event.seq).await?;
            processes.retain(|id,_|!reaped.contains(id));
        }
        for process in retained.processes {
            let belongs:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND operation_id=?)")
                .bind(session.as_ref()).bind(process.creator_turn_id.as_ref()).fetch_one(&mut **tx).await?;
            if !belongs || processes.get(&process.process_id).is_some_and(|previous|previous!=&process) {
                return Err(SessionStoreError::InvalidPayload("Session process retention changed its frozen creator identity".into()));
            }
            processes.insert(process.process_id.clone(),process);
        }
        if processes.len()>64 {return Err(SessionStoreError::InvalidPayload("Session process owner exceeds its live or unconfirmed scope budget".into()));}
        last=Some(event.event_id);
    }
    }
    let reaped=super::session_process_retirement::confirmed_tx(tx,session,&processes,through).await?;
    processes.retain(|id,_|!reaped.contains(id));
    if processes.len()>64 {return Err(SessionStoreError::InvalidPayload("Session process owner exceeds its live or unconfirmed scope budget".into()));}
    Ok((processes,last))
}

impl AgentSessionStore {
    /// An in-memory replacement must never adopt a previous owner's PID or
    /// silently relaunch work whose Session retention was not released.
    pub async fn unreleased_session_process_generations(&self,owner:&PrincipalRef,session:&AgentSessionId)
        ->Result<BTreeSet<String>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let live=retained_session_tx(&mut tx,session).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let mut generations=BTreeSet::new();
        let mut cursor=0i64;
        loop {
        // Filter only canonical owner metadata before hydrating a page. File,
        // media and model output bodies are not process ownership evidence.
        let rows:Vec<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
            FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
            WHERE e.session_id=? AND e.seq>? AND (e.kind='session/process-owner-released' \
              OR (e.kind='runtime/progress-recorded' \
                AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='host_cleanup_proven' \
                AND COALESCE(json_type(e.inline_json,'$.event.session_process_retention'),json_type(CAST(p.body AS TEXT),'$.value.event.session_process_retention')) IS NOT NULL)) \
            ORDER BY e.seq LIMIT 32")
            .bind(session.as_ref()).bind(cursor).fetch_all(&mut *tx).await?;
        if rows.is_empty(){break;}
        for row in rows {
            cursor=cursor.max(row.seq);
            let event=event_from_row(row)?;let payload=payload_value_for_event_tx(&mut tx,&event).await?;
            if event.kind.0==SESSION_PROCESS_OWNER_RELEASED {
                let released:SessionProcessOwnerReleasedV1=serde_json::from_value(payload)?;
                validate_release_tx(&mut tx,&event,&released).await?;
                generations.remove(&released.retention.owner_generation);
            } else if payload.pointer("/event/event").and_then(Value::as_str)==Some("host_cleanup_proven")
                && let Some(value)=payload.pointer("/event/session_process_retention") {
                let retention:NativeSessionProcessRetention=serde_json::from_value(value.clone())?;
                validate_retention_tx(&mut tx,&event,&retention).await?;
                if !retention.processes.is_empty(){generations.insert(retention.owner_generation);}
            }
        }
        }
        let mut unresolved=BTreeSet::new();
        for generation in generations {
            if !retained_tx(&mut tx,session,&generation,i64::MAX as u64).await?.0.is_empty(){unresolved.insert(generation);}
        }
        tx.commit().await?;Ok(unresolved)
    }

    /// Read the exact canonical union before a live owner releases it. This
    /// observation grants neither process I/O nor quiescence by itself.
    pub async fn session_process_owner_retention(&self,owner:&PrincipalRef,session:&AgentSessionId,generation:&str)
        ->Result<Option<NativeSessionProcessRetention>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let live=retained_session_tx(&mut tx,session).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let (processes,_)=retained_tx(&mut tx,session,generation,i64::MAX as u64).await?;
        let retention=if processes.is_empty(){None}else{Some(NativeSessionProcessRetention {agent_session_id:session.clone(),owner_ref:owner.clone(),
            owner_generation:generation.into(),processes:processes.into_values().collect()})};
        tx.commit().await?;Ok(retention)
    }

    /// Called only after the exact live owner has closed admission and reaped
    /// every registered tree. The receipt covers exactly the durable union;
    /// it cannot grant cleanup of unrecorded or foreign processes.
    pub async fn record_session_process_owner_released(&self,owner:&PrincipalRef,mut receipt:SessionProcessOwnerReleasedV1)
        -> Result<Option<SessionEventAck>,SessionStoreError> {
        receipt.validate().map_err(SessionStoreError::InvalidPayload)?;
        if &receipt.retention.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let session=receipt.retention.agent_session_id.clone();
        let key=release_key(&session,&receipt.retention.owner_generation);
        let mut tx=self.begin_write_transaction().await?;
        let live=retained_session_tx(&mut tx,&session).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let supplied:BTreeMap<_,_>=receipt.retention.processes.iter().map(|process|(process.process_id.clone(),process.clone())).collect();
        if let Some(row)=event_by_event_id_tx(&mut tx,&key).await? {
            let event=event_from_row(row)?;
            let original:SessionProcessOwnerReleasedV1=serde_json::from_value(payload_value_for_event_tx(&mut tx,&event).await?)?;
            if original.retention.owner_ref!=*owner || original.retention.owner_generation!=receipt.retention.owner_generation
                || original.retention.processes.len()!=supplied.len()
                || original.retention.processes.iter().any(|process| supplied.get(&process.process_id)!=Some(process)) {
                return Err(SessionStoreError::IdempotencyConflict("Session process release differs from its original owner receipt".into()));
            }
            let ack=event_ack(&event);tx.commit().await?;return Ok(Some(ack));
        }
        let (retained,last)=retained_tx(&mut tx,&session,&receipt.retention.owner_generation,i64::MAX as u64).await?;
        if retained.is_empty() {return Err(SessionStoreError::InvalidPayload("Session process release has no original retained owner generation".into()));}
        if retained.len()!=supplied.len() || retained.iter().any(|(id,process)|supplied.get(id)!=Some(process)) {
            return Err(SessionStoreError::InvalidPayload("Session process release does not cover its exact retained creator/resource set".into()));
        }
        receipt.retention.processes=retained.into_values().collect();
        let append=SessionEventAppend {agent_session_id:session.clone(),event_id:key.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind(SESSION_PROCESS_OWNER_RELEASED.into()),kind_version:1,
                correlation_id:session.as_ref().into(),causation_event_id:last,payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(&receipt)?))}};
        let ack=required_ack(self.append_event_tx_with_policy(&mut tx,&append,None,AppendSessionStatePolicy::ProcessOwnerRelease).await?)?;
        tx.commit().await?;Ok(Some(ack))
    }
}

pub(super) async fn validate_release_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,receipt:&SessionProcessOwnerReleasedV1)
    ->Result<(),SessionStoreError> {
    receipt.validate().map_err(SessionStoreError::InvalidPayload)?;
    let retained=&receipt.retention;
    let live=retained_session_tx(tx,&event.agent_session_id).await?;
    if retained.agent_session_id!=event.agent_session_id || retained.owner_ref!=live.owner_ref || event.producer_id.as_ref()!="session_api"
        || event.kind.0!=SESSION_PROCESS_OWNER_RELEASED || event.kind_version!=1 || event.correlation_id.as_ref()!=event.agent_session_id.as_ref()
        || event.event_id.as_ref()!=release_key(&event.agent_session_id,&retained.owner_generation) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let (original,last)=retained_tx(tx,&event.agent_session_id,&retained.owner_generation,event.seq).await?;
    if original.into_values().collect::<Vec<_>>()!=retained.processes || last!=event.causation_event_id {
        return Err(SessionStoreError::InvalidPayload("Session process release differs from the original canonical retention set".into()));
    }
    Ok(())
}
