//! The real Session process owner's release receipt. No table, PID probing,
//! reconstructed process, checkpoint conversion, or second ownership ledger.
use super::*;
use nomifun_agent_contracts::{NativeRetainedProcess, NativeSessionProcessRetention, SessionProcessOwnerReleasedV1, SESSION_PROCESS_OWNER_RELEASED,
    ResolvedSnapshotRef,DigestHex,ActionId};

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
            let reaped=super::session_process_retirement::confirmed_tx(tx,session,generation,&processes,event.seq).await?;
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
    let reaped=super::session_process_retirement::confirmed_tx(tx,session,generation,&processes,through).await?;
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
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
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
            let ack=event_ack(&event);db_write_observation.commit(tx).await?;return Ok(Some(ack));
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
        db_write_observation.commit(tx).await?;Ok(Some(ack))
    }
}

fn reaped_key(receipt:&nomifun_agent_contracts::SessionProcessReapedV1)->String {
    format!("process-reaped:{}:{}:{}",receipt.retention.agent_session_id.as_ref(),receipt.retention.owner_generation,
        receipt.retention.processes[0].process_id)
}

async fn original_process_retention_tx(tx:&mut Transaction<'_,Sqlite>,retention:&NativeSessionProcessRetention,through:u64)
    ->Result<Option<SessionEventRecord>,SessionStoreError> {
    let process=&retention.processes[0];
    let row:Option<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
        FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND e.kind='runtime/progress-recorded' AND e.seq<? \
          AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='host_cleanup_proven' \
          AND COALESCE(json_extract(e.inline_json,'$.event.session_process_retention.owner_generation'),json_extract(CAST(p.body AS TEXT),'$.value.event.session_process_retention.owner_generation'))=? \
          AND EXISTS(SELECT 1 FROM json_each(COALESCE(json_extract(e.inline_json,'$.event.session_process_retention.processes'), \
            json_extract(CAST(p.body AS TEXT),'$.value.event.session_process_retention.processes'))) process WHERE json_extract(process.value,'$.process_id')=?) \
        ORDER BY e.seq DESC LIMIT 1")
        .bind(retention.agent_session_id.as_ref()).bind(as_i64(through,"process source cursor")?).bind(&retention.owner_generation)
        .bind(&process.process_id).fetch_optional(&mut **tx).await?;
    row.map(event_from_row).transpose()
}

/// Before a Turn ends there may be no whole-Turn retention yet. The original
/// returned start and its pre-dispatch scope binding can authorize only this
/// native reap, even while unrelated Effects remain pending or unknown.
async fn native_launch_cause_tx(tx:&mut Transaction<'_,Sqlite>,retention:&NativeSessionProcessRetention,through:u64)
    ->Result<EventId,SessionStoreError> {
    let process=&retention.processes[0];
    let launch=super::session_process_retirement::running_launch_tx(tx,&retention.agent_session_id,process,through).await?
        .ok_or_else(||SessionStoreError::Conflict("original process retention or returned launch is not committed yet".into()))?;
    let dispatch=event_by_event_id_tx(tx,launch.dispatch_event_id.as_ref()).await?.map(event_from_row).transpose()?
        .ok_or(SessionStoreError::ExecutionFenced)?;
    let started_id:Option<String>=sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(retention.agent_session_id.as_ref()).bind(process.creator_turn_id.as_ref()).fetch_optional(&mut **tx).await?;
    let started=event_by_event_id_tx(tx,&started_id.ok_or(SessionStoreError::ExecutionFenced)?).await?.map(event_from_row).transpose()?
        .ok_or(SessionStoreError::ExecutionFenced)?;
    if started.kind.0!="turn/started"||started.kind_version!=1||started.correlation_id.as_ref()!=process.creator_turn_id.as_ref()
        ||started.seq>=dispatch.seq||dispatch.causation_event_id!=started.causation_event_id
        ||launch.settlement.causation_event_id!=dispatch.causation_event_id
        ||payload_value_for_event_tx(tx,&started).await?.get("resolved_snapshot_ref")!=Some(&serde_json::to_value(&process.resolved_snapshot_ref)?) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let dispatch_payload=payload_value_for_event_tx(tx,&dispatch).await?;
    let call=dispatch_payload.pointer("/event/dispatch/call_id").and_then(Value::as_str).ok_or(SessionStoreError::ExecutionFenced)?;
    let rows:Vec<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
        FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND e.seq>? AND e.seq<? AND e.kind='runtime/progress-recorded' AND e.kind_version=1 AND e.producer_id='runtime_supervisor' \
          AND e.correlation_id=? AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='host_process_dispatch' \
          AND COALESCE(json_extract(e.inline_json,'$.event.operation_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.operation_id'))=? ORDER BY e.seq LIMIT 2")
        .bind(retention.agent_session_id.as_ref()).bind(as_i64(dispatch.seq,"process dispatch cursor")?)
        .bind(as_i64(launch.settlement.seq,"process launch cursor")?).bind(process.creator_turn_id.as_ref()).bind(process.creator_call_id.as_ref())
        .fetch_all(&mut **tx).await?;
    if rows.len()!=1 {return Err(SessionStoreError::ExecutionFenced);}
    let witness_event=event_from_row(rows.into_iter().next().expect("one original process dispatch"))?;
    if witness_event.causation_event_id!=dispatch.causation_event_id {return Err(SessionStoreError::ExecutionFenced);}
    let witness_payload=payload_value_for_event_tx(tx,&witness_event).await?;
    let witness:nomifun_agent_contracts::NativeProcessWitness=serde_json::from_value(witness_payload.get("event").cloned().ok_or(SessionStoreError::ExecutionFenced)?)?;
    witness.validate().map_err(|error|SessionStoreError::InvalidPayload(error.into()))?;
    let nomifun_agent_contracts::NativeProcessWitness::Dispatch {process_owner:Some(binding),..}=witness else {return Err(SessionStoreError::ExecutionFenced);};
    if binding.owner_generation!=retention.owner_generation||binding.resource_binding_digest!=process.resource_binding_digest {
        return Err(SessionStoreError::ExecutionFenced);
    }
    // Reuse the original Effect lifecycle and native tool causation. A fake
    // Running JSON value, rejected/unknown start or another creator never
    // acquires a native cleanup authority through this observation writer.
    let effects:Vec<StoredEffectRow>=sqlx::query_as("SELECT effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id,resource_binding_id,resource_key,input_digest,strategy,state,started_event_id,terminal_event_id,created_at,settled_at \
        FROM agent_effects WHERE session_id=? AND turn_id=? AND operation_id=? AND owner_domain='workspace' AND capability_module='workspace.process' AND action_id='workspace.process/start' LIMIT 2")
        .bind(retention.agent_session_id.as_ref()).bind(process.creator_turn_id.as_ref()).bind(process.creator_call_id.as_ref()).fetch_all(&mut **tx).await?;
    if effects.len()!=1 {return Err(SessionStoreError::ExecutionFenced);}
    let effect=&effects[0];
    let returned=terminal_content::effect_observation(tx,effect).await?.ok_or(SessionStoreError::ExecutionFenced)?;
    if effect.state!="returned"||effect.strategy!="managed_effect"
        ||returned.pointer("/result/state").and_then(Value::as_str)!=Some("running")
        ||returned.pointer("/result/process_id").and_then(Value::as_str)!=Some(process.process_id.as_str())
        ||returned.pointer("/result/pid").and_then(Value::as_u64)!=Some(u64::from(process.pid)) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let effect_start=event_by_event_id_tx(tx,&effect.started_event_id).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    let effect_terminal=event_by_event_id_tx(tx,effect.terminal_event_id.as_deref().ok_or(SessionStoreError::ExecutionFenced)?).await?
        .map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    let tool=event_by_event_id_tx(tx,effect_start.causation_event_id.as_ref().ok_or(SessionStoreError::ExecutionFenced)?.as_ref()).await?
        .map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    if tool.kind.0!="tool/call-started"||tool.kind_version!=1||tool.producer_id.as_ref()!="runtime_supervisor"
        ||tool.agent_session_id!=retention.agent_session_id||tool.causation_event_id.as_ref()!=Some(&started.event_id)
        ||tool.seq<=dispatch.seq||effect_start.seq<=tool.seq||witness_event.seq<=effect_start.seq
        ||effect_terminal.seq<=witness_event.seq||launch.settlement.seq<=effect_terminal.seq
        ||effect_terminal.causation_event_id.as_ref()!=Some(&effect_start.event_id) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let tool_payload=payload_value_for_event_tx(tx,&tool).await?;
    for (field,expected) in [("operation_id",process.creator_call_id.as_ref()),("call_id",call),
        ("capability_id","workspace.process"),("action_id","workspace.process/start")] {
        if tool_payload.get(field).and_then(Value::as_str)!=Some(expected) {return Err(SessionStoreError::ExecutionFenced);}
    }
    Ok(launch.settlement.event_id)
}

pub(super) async fn validate_reaped_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,
    receipt:&nomifun_agent_contracts::SessionProcessReapedV1)->Result<(),SessionStoreError> {
    receipt.validate().map_err(SessionStoreError::InvalidPayload)?;
    let retention=&receipt.retention;
    let live=retained_session_tx(tx,&event.agent_session_id).await?;
    if retention.agent_session_id!=event.agent_session_id||retention.owner_ref!=live.owner_ref||event.producer_id.as_ref()!="session_api"
        ||event.kind.0!=nomifun_agent_contracts::SESSION_PROCESS_REAPED||event.kind_version!=1
        ||event.correlation_id.as_ref()!=event.agent_session_id.as_ref()||event.event_id.as_ref()!=reaped_key(receipt) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let process=&retention.processes[0];
    let referenced=super::session_process_retirement::terminal_tx(tx,&event.agent_session_id,process,event.seq).await?;
    let metadata=if let Some(archive)=&receipt.output_archive {
        let metadata=session_process_archive::terminal(archive,&reaped_key(receipt))?;
        if let Some(reference)=&receipt.terminal_ref {
            let (_,poll)=referenced.as_ref().filter(|(id,_)|id==reference)
                .ok_or_else(||SessionStoreError::InvalidPayload("archived terminal reference is not its original first-party terminal".into()))?;
            if poll.terminal_metadata().as_ref()!=Some(&metadata) {return Err(SessionStoreError::ExecutionFenced);}
        }
        metadata
    }else{
    let poll:nomifun_engine_core::EngineProcessPoll=match (&receipt.terminal,&receipt.terminal_ref) {
        (Some(value),None)=>serde_json::from_value(value.0.clone())?,
        (None,Some(reference))=>referenced.as_ref().filter(|(id,_)|id==reference).map(|(_,poll)|poll.clone())
            .ok_or_else(||SessionStoreError::InvalidPayload("process terminal reference is not its original first-party terminal".into()))?,
        _=>return Err(SessionStoreError::ExecutionFenced),
    };
    poll.terminal_metadata().ok_or_else(||SessionStoreError::InvalidPayload("process reap requires its known original native terminal and complete cleanup".into()))?
    };
    if !metadata.is_confirmed_reaped() {return Err(SessionStoreError::ExecutionFenced);}
    // Historical physical reap is portable. The original writer validated its
    // native archive; an unavailable decoder on this host must not resurrect
    // native ownership or prevent clear/delete. Output reads validate again.
    let cause=if let Some(original)=original_process_retention_tx(tx,retention,event.seq).await? {
        let source:NativeSessionProcessRetention=serde_json::from_value(payload_value_for_event_tx(tx,&original).await?
            .pointer("/event/session_process_retention").cloned().ok_or_else(||SessionStoreError::InvalidPayload("process retention body missing".into()))?)?;
        validate_retention_tx(tx,&original,&source).await?;
        if source.owner_ref!=retention.owner_ref||source.owner_generation!=retention.owner_generation||!source.processes.contains(process) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        original.event_id
    } else if let Some(reference)=&receipt.terminal_ref {
        reference.clone()
    } else {
        if receipt.output_archive.is_none()&&receipt.output_replay.is_none() {return Err(SessionStoreError::ExecutionFenced);}
        native_launch_cause_tx(tx,retention,event.seq).await?
    };
    if event.causation_event_id.as_ref()!=Some(&cause) {return Err(SessionStoreError::ExecutionFenced);}
    let started:Option<String>=sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(event.agent_session_id.as_ref()).bind(process.creator_turn_id.as_ref()).fetch_optional(&mut **tx).await?;
    let started=event_by_event_id_tx(tx,&started.ok_or(SessionStoreError::ExecutionFenced)?).await?.map(event_from_row).transpose()?
        .ok_or(SessionStoreError::ExecutionFenced)?;
    if payload_value_for_event_tx(tx,&started).await?.get("resolved_snapshot_ref")!=Some(&serde_json::to_value(&process.resolved_snapshot_ref)?) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    if event_by_event_id_tx(tx,&release_key(&event.agent_session_id,&retention.owner_generation)).await?
        .is_some_and(|released|released.seq<event.seq as i64) {return Err(SessionStoreError::ExecutionFenced);}
    let launched=super::session_process_retirement::launched_tx(tx,&event.agent_session_id,
        &BTreeMap::from([(process.process_id.clone(),process.clone())]),event.seq).await?;
    if !launched.contains(&process.process_id) {return Err(SessionStoreError::InvalidPayload("process reap differs from its first-party launch".into()));}
    Ok(())
}

impl AgentSessionStore {
    /// Repeating an exact host resource Stop after the original scope retired
    /// reads its actual canonical reap; it never selects a successor process.
    pub async fn session_process_resource_stopped(&self,owner:&PrincipalRef,session:&AgentSessionId,generation:&str,process_id:&str,
        creator_turn:&str,creator_call:&str)->Result<bool,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let live=retained_session_tx(&mut tx,session).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let key=format!("process-reaped:{}:{generation}:{process_id}",session.as_ref());
        let Some(row)=event_by_event_id_tx(&mut tx,&key).await? else {return Ok(false);};
        let event=event_from_row(row)?;
        let receipt:nomifun_agent_contracts::SessionProcessReapedV1=serde_json::from_value(payload_value_for_event_tx(&mut tx,&event).await?)?;
        validate_reaped_tx(&mut tx,&event,&receipt).await?;
        let original=&receipt.retention.processes[0];
        let exact=receipt.retention.owner_generation==generation&&original.process_id==process_id
            &&original.creator_turn_id.as_ref()==creator_turn&&original.creator_call_id.as_ref()==creator_call;
        tx.commit().await?;Ok(exact)
    }

    /// Read the original terminal after its live handle retired. This is only
    /// observation; old Snapshot/resources never authorize new native work.
    pub async fn session_process_terminal(&self,owner:&PrincipalRef,session:&AgentSessionId,process_id:&str,
        current_snapshot:&ResolvedSnapshotRef,current_resource_digest:&DigestHex,action:&ActionId,cursor:u64)
        ->Result<Option<(nomifun_engine_core::EngineProcessPoll,Option<StrictJsonValue>)>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let live=retained_session_tx(&mut tx,session).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let row:Option<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
            FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
            WHERE e.session_id=? AND e.kind='session/process-reaped' \
              AND COALESCE(json_extract(e.inline_json,'$.retention.processes[0].process_id'),json_extract(CAST(p.body AS TEXT),'$.value.retention.processes[0].process_id'))=? \
            ORDER BY e.seq DESC LIMIT 1")
            .bind(session.as_ref()).bind(process_id).fetch_optional(&mut *tx).await?;
        let Some(row)=row else {return Ok(None);};
        let event=event_from_row(row)?;
        let receipt:nomifun_agent_contracts::SessionProcessReapedV1=serde_json::from_value(payload_value_for_event_tx(&mut tx,&event).await?)?;
        validate_reaped_tx(&mut tx,&event,&receipt).await?;
        let process=&receipt.retention.processes[0];
        if &process.resource_binding_digest!=current_resource_digest {return Err(SessionStoreError::ExecutionFenced);}
        if &process.resolved_snapshot_ref!=current_snapshot {
            let raw:Option<String>=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=? AND snapshot_digest=?")
                .bind(process.resolved_snapshot_ref.snapshot_id.as_ref()).bind(process.resolved_snapshot_ref.snapshot_digest.as_ref())
                .fetch_optional(&mut *tx).await?;
            let original:nomifun_agent_contracts::ResolvedSnapshotEnvelope=serde_json::from_str(&raw
                .ok_or_else(||SessionStoreError::InvalidPayload("original process Snapshot is unavailable".into()))?)?;
            original.validate().map_err(|error|SessionStoreError::InvalidPayload(error.message))?;
            if original.snapshot_ref!=process.resolved_snapshot_ref||!original.content.enabled_capabilities.iter()
                .any(|capability|capability.capability.id.as_ref()=="workspace.process"&&capability.action_allowlist.contains(action)) {
                return Err(SessionStoreError::ExecutionFenced);
            }
        }
        if receipt.output_archive.is_some() {
            let poll=session_process_archive::read_tx(&mut tx,&receipt,&reaped_key(&receipt),cursor).await?;
            tx.commit().await?;return Ok(Some((poll,receipt.component_identity)));
        }
        let poll=match (&receipt.terminal,&receipt.terminal_ref) {
            (Some(value),None)=>serde_json::from_value(value.0.clone())?,
            (None,Some(reference))=>super::session_process_retirement::terminal_tx(&mut tx,session,process,event.seq).await?
                .filter(|(id,_)|id==reference).map(|(_,poll)|poll).ok_or(SessionStoreError::ExecutionFenced)?,
            _=>return Err(SessionStoreError::ExecutionFenced),
        };
        let poll=if let Some(replay)=&receipt.output_replay {
            let replay:nomifun_engine_core::EngineProcessOutputReplay=serde_json::from_value(replay.0.clone())?;
            poll.replay_output(&replay,cursor).map_err(|error|SessionStoreError::InvalidPayload(error.to_string()))?
        } else {poll};
        tx.commit().await?;Ok(Some((poll,receipt.component_identity)))
    }

    /// Called by the original retained native owner after a measured terminal.
    /// Preserve its complete bounded output atomically with this exact receipt;
    /// failing this write never permits releasing the last native witness.
    pub async fn record_session_process_reaped(&self,owner:&PrincipalRef,receipt:nomifun_agent_contracts::SessionProcessReapedV1)
        ->Result<SessionEventAck,SessionStoreError> {
        self.record_session_process_reaped_inner(owner,receipt,None).await
    }

    /// Keep bounded native bytes in their original typed carrier. Large valid
    /// output must not first expand to millions of generic JSON Value nodes.
    pub async fn record_session_process_reaped_with_output(&self,owner:&PrincipalRef,receipt:nomifun_agent_contracts::SessionProcessReapedV1,
        output:&nomifun_engine_core::EngineProcessOutputReplay)->Result<SessionEventAck,SessionStoreError> {
        self.record_session_process_reaped_inner(owner,receipt,Some(output)).await
    }

    #[tracing::instrument(name = "canonical.process_reaped", skip_all, fields(session_id = %receipt.retention.agent_session_id.as_ref()))]
    async fn record_session_process_reaped_inner(&self,owner:&PrincipalRef,mut receipt:nomifun_agent_contracts::SessionProcessReapedV1,
        native_output:Option<&nomifun_engine_core::EngineProcessOutputReplay>)->Result<SessionEventAck,SessionStoreError> {
        receipt.validate().map_err(SessionStoreError::InvalidPayload)?;
        if receipt.output_archive.is_some() {return Err(SessionStoreError::InvalidPayload("native archive references are authored only by the original receipt writer".into()));}
        if &receipt.retention.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        if native_output.is_some()&&receipt.output_replay.is_some() {return Err(SessionStoreError::InvalidPayload("native output must have exactly its original typed carrier".into()));}
        let inline_output=receipt.output_replay.as_ref().map(|replay|serde_json::from_value::<nomifun_engine_core::EngineProcessOutputReplay>(replay.0.clone())).transpose()?;
        let output=native_output.or(inline_output.as_ref());
        if let Some(replay)=output {
            replay.validate().map_err(|error|SessionStoreError::InvalidPayload(error.to_string()))?;
            if let Some(terminal)=&receipt.terminal {
                let poll:nomifun_engine_core::EngineProcessPoll=serde_json::from_value(terminal.0.clone())?;
                let complete=poll.replay_output(&replay,0).map_err(|error|SessionStoreError::InvalidPayload(error.to_string()))?;
                if complete!=poll {return Err(SessionStoreError::InvalidPayload("process terminal differs from its frozen output".into()));}
            }
        }
        let archive_bytes=output.map(serde_json::to_vec).transpose()?;
        let key=reaped_key(&receipt);let session=receipt.retention.agent_session_id.clone();
        let db_observation=nomifun_db::DatabaseOperationGuard::new(&self.pool);
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        db_observation.acquired();
        let live=retained_session_tx(&mut tx,&session).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        if let Some(row)=event_by_event_id_tx(&mut tx,&key).await? {
            let event=event_from_row(row)?;
            let original:nomifun_agent_contracts::SessionProcessReapedV1=serde_json::from_value(payload_value_for_event_tx(&mut tx,&event).await?)?;
            let mut original_chunks=Vec::new();
            if original.output_archive.is_some() {
                let poll=match &receipt.terminal {
                    Some(terminal)=>serde_json::from_value::<nomifun_engine_core::EngineProcessPoll>(terminal.0.clone())?,
                    None=>super::session_process_retirement::terminal_tx(&mut tx,&session,&receipt.retention.processes[0],event.seq).await?
                        .filter(|(id,_)|Some(id)==receipt.terminal_ref.as_ref()).map(|(_,poll)|poll).ok_or(SessionStoreError::ExecutionFenced)?,
                };
                if let Some(reference)=&original.terminal_ref {receipt.terminal_ref=Some(reference.clone());}
                // Deterministic original references/digests, with no native
                // work or new payload insertion on a receipt retry.
                original_chunks=session_process_archive::prepare(&mut receipt,&poll,&key,archive_bytes.as_deref())?;
            } else if let (Some(reference),Some(supplied))=(&original.terminal_ref,&receipt.terminal) {
                let (_,poll)=super::session_process_retirement::terminal_tx(&mut tx,&session,&original.retention.processes[0],event.seq).await?
                    .filter(|(id,_)|id==reference).ok_or(SessionStoreError::ExecutionFenced)?;
                let poll=if let Some(replay)=&original.output_replay {
                    let replay:nomifun_engine_core::EngineProcessOutputReplay=serde_json::from_value(replay.0.clone())?;
                    poll.replay_output(&replay,0).map_err(|error|SessionStoreError::InvalidPayload(error.to_string()))?
                }else{poll};
                if poll!=serde_json::from_value::<nomifun_engine_core::EngineProcessPoll>(supplied.0.clone())? {
                    return Err(SessionStoreError::IdempotencyConflict("process reap changed its original terminal".into()));
                }
                receipt.terminal=None;receipt.terminal_ref=Some(reference.clone());
            }
            if original.output_archive.is_none()&&native_output.is_some() {
                receipt.output_replay=native_output.map(serde_json::to_value).transpose()?.map(StrictJsonValue);
            }
            if original!=receipt {return Err(SessionStoreError::IdempotencyConflict("process reap changed its original native receipt".into()));}
            // A portable historical reap remains true if observation bytes
            // later become unavailable. A live owner about to release its
            // last witness still requires the original archive to be intact.
            for chunk in &original_chunks {
                let stored=payload_by_id_tx(&mut tx,chunk.payload_id.as_ref()).await?.map(payload_from_row).transpose()?;
                if stored.as_ref()!=Some(chunk) {return Err(SessionStoreError::InvalidPayload("original native output archive is missing or differs from its committed receipt".into()));}
            }
            validate_reaped_tx(&mut tx,&event,&original).await?;
            let ack=event_ack(&event);db_write_observation.commit(tx).await?;return Ok(ack);
        }
        let native_terminal=match &receipt.terminal {
            Some(terminal)=>serde_json::from_value::<nomifun_engine_core::EngineProcessPoll>(terminal.0.clone())?,
            None=>super::session_process_retirement::terminal_tx(&mut tx,&session,&receipt.retention.processes[0],i64::MAX as u64).await?
                .filter(|(id,_)|Some(id)==receipt.terminal_ref.as_ref()).map(|(_,poll)|poll).ok_or(SessionStoreError::ExecutionFenced)?,
        };
        if let Some((reference,_))=super::session_process_retirement::terminal_tx(&mut tx,&session,&receipt.retention.processes[0],i64::MAX as u64).await? {
            receipt.terminal=None;receipt.terminal_ref=Some(reference);
        }
        let source=match original_process_retention_tx(&mut tx,&receipt.retention,i64::MAX as u64).await? {
            Some(source)=>source.event_id,
            None=>match &receipt.terminal_ref {
                Some(reference)=>reference.clone(),
                None if output.is_some()=>native_launch_cause_tx(&mut tx,&receipt.retention,i64::MAX as u64).await?,
                None=>return Err(SessionStoreError::Conflict("original process retention is not committed yet".into())),
            },
        };
        let chunks=session_process_archive::prepare(&mut receipt,&native_terminal,&key,archive_bytes.as_deref())?;
        for chunk in &chunks {insert_payload_with_policy_tx(&mut tx,chunk,true).await?;}
        let value=serde_json::to_value(&receipt)?;let bytes=canonical_json_bytes(&value)?;
        let payload=SessionPayloadRecord {payload_id:format!("{key}:payload").into(),agent_session_id:session.clone(),media_type:session_process_archive::RECEIPT_MEDIA.into(),
            byte_len:bytes.len() as u64,digest:digest_bytes(&bytes),body:SessionPayloadBody::Json(StrictJsonValue(value))};
        insert_payload_with_policy_tx(&mut tx,&payload,true).await?;
        let append=SessionEventAppend {agent_session_id:session.clone(),event_id:key.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind(nomifun_agent_contracts::SESSION_PROCESS_REAPED.into()),kind_version:1,
                correlation_id:session.as_ref().into(),causation_event_id:Some(source),payload:SessionEventPayloadRef::Stored(payload.payload_id.clone())}};
        let ack=required_ack(self.append_event_tx_with_policy(&mut tx,&append,None,AppendSessionStatePolicy::ProcessOwnerRelease).await?)?;
        db_write_observation.commit(tx).await?;Ok(ack)
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
