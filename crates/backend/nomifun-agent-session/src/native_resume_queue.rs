//! A continuation intent in the original Event chain. It does not authorize
//! native execution, consume budget or create an Attempt while capacity waits.
use super::*;

#[derive(Clone, Debug, serde::Serialize)]
pub struct NativeResumeRequestReceipt {
    pub operation_id: OperationId,
    pub request_event_id: EventId,
    pub seq: u64,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub struct PendingNativeResumeRequest {
    pub rowid: i64,
    pub agent_session_id: AgentSessionId,
    pub owner: PrincipalRef,
    pub request: NativeResumeRequest,
    pub receipt: NativeResumeRequestReceipt,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NativeResumeCommandReceipt {
    Authorized { #[serde(flatten)] receipt: NativeResumeReceipt },
    Queued { #[serde(flatten)] receipt: NativeResumeRequestReceipt },
}

pub(super) fn admission_error(error: nomifun_db::DbError) -> SessionStoreError {
    match error {
        nomifun_db::DbError::Query(error) => SessionStoreError::Sqlite(error),
        nomifun_db::DbError::Conflict(message) | nomifun_db::DbError::NotFound(message) => SessionStoreError::Conflict(message),
        other => SessionStoreError::InvalidSession(other.to_string()),
    }
}

async fn request_is_current_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
    request: &NativeResumeRequest) -> Result<bool, SessionStoreError> {
    let row: Option<(String, Option<String>, String, Option<String>, i64, Option<String>)> = sqlx::query_as(
        "SELECT head.status,head.active_turn_id,turn.state,turn.native_pause_json,turn.native_checkpoint_revision,turn.native_checkpoint_digest \
         FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id \
         WHERE turn.session_id=? AND turn.operation_id=? AND turn.terminal_event_id IS NULL",
    ).bind(session.as_ref()).bind(request.operation_id.as_ref()).fetch_optional(&mut **tx).await?;
    let Some((head, active, state, Some(raw), revision, digest)) = row else { return Ok(false); };
    let pause: NativePauseState = serde_json::from_str(&raw)?;
    let current = head == "paused" && active.as_deref() == Some(request.operation_id.as_ref()) && state == "running"
        && pause.revision == request.expected_pause_revision && revision >= 0
        && revision as u64 == request.expected_checkpoint_revision
        && digest.as_deref() == Some(request.expected_checkpoint_digest.as_ref());
    Ok(current && target_is_current_tx(tx,session,&request.operation_id).await?)
}

async fn target_is_current_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
    operation: &OperationId) -> Result<bool, SessionStoreError> {
    let owner=live_session_by_id_tx(tx,session.as_ref()).await?.owner_ref;
    nomifun_db::native_resume_target_eligible_tx(tx,&owner.principal_id,session.as_ref(),operation.as_ref())
        .await.map_err(admission_error)
}

async fn request_record_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
    owner: &PrincipalRef, request: &NativeResumeRequest) -> Result<Option<SessionEventRecord>, SessionStoreError> {
    let id = native_pause::native_key("resume-request", session, &request.idempotency_key)?;
    let Some(row) = event_by_event_id_tx(tx, &id).await? else { return Ok(None); };
    let event = event_from_row(row)?;
    let payload = payload_value_for_event_tx(tx, &event).await?;
    if event.agent_session_id != *session || event.correlation_id.as_ref() != request.operation_id.as_ref()
        || event.kind.0 != "turn/resume-requested" || event.kind_version != 1 || event.producer_id.as_ref() != "runtime_supervisor"
        || payload.get("request_digest") != Some(&serde_json::to_value(digest_payload(request)?)?)
        || payload.get("owner_digest") != Some(&serde_json::to_value(digest_payload(owner)?)?) {
        return Err(SessionStoreError::IdempotencyConflict("queued native resume target or request changed".into()));
    }
    Ok(Some(event))
}

async fn blocked_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, event: &EventId)
    -> Result<bool, SessionStoreError> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='turn/resume-blocked' AND causation_event_id=?)")
        .bind(session.as_ref()).bind(event.as_ref()).fetch_one(&mut **tx).await?)
}

pub(super) async fn ensure_request_not_blocked_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
    owner: &PrincipalRef, request: &NativeResumeRequest) -> Result<(), SessionStoreError> {
    if let Some(event) = request_record_tx(tx,session,owner,request).await?
        && blocked_tx(tx,session,&event.event_id).await? {
        return Err(SessionStoreError::Conflict("the original queued continuation is blocked; submit a new explicit request after resolving its reason".into()));
    }
    Ok(())
}

impl AgentSessionStore {
    pub async fn native_resume_request_receipt(&self, owner: &PrincipalRef, session: &AgentSessionId,
        request: &NativeResumeRequest) -> Result<Option<NativeResumeRequestReceipt>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let Some(event) = request_record_tx(&mut tx,session,owner,request).await? else { return Ok(None); };
        if !request_is_current_tx(&mut tx,session,request).await? || blocked_tx(&mut tx,session,&event.event_id).await? { return Ok(None); }
        tx.commit().await?;
        Ok(Some(NativeResumeRequestReceipt { operation_id: request.operation_id.clone(), request_event_id: event.event_id,
            seq:event.seq,duplicate:true }))
    }

    pub async fn request_native_resume(&self, owner: &PrincipalRef, session: &AgentSessionId,
        request: &NativeResumeRequest) -> Result<NativeResumeRequestReceipt, SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let previous = request_record_tx(&mut tx,session,owner,request).await?;
        if !request_is_current_tx(&mut tx,session,request).await? { return Err(SessionStoreError::ExecutionFenced); }
        if let Some(event) = previous {
            if blocked_tx(&mut tx,session,&event.event_id).await? { return Err(SessionStoreError::Conflict("the original continuation request is blocked; inspect its control result".into())); }
            tx.commit().await?;
            return Ok(NativeResumeRequestReceipt { operation_id:request.operation_id.clone(),request_event_id:event.event_id,seq:event.seq,duplicate:true });
        }
        // Also fences a Stop that won the writer after an earlier capacity
        // probe. A false result accepts waiting; an invalid range is rejected.
        nomifun_db::native_resume_capacity_available_tx(
            &mut tx,&owner.principal_id,session.as_ref(),request.operation_id.as_ref()).await.map_err(admission_error)?;
        let cause = format!("native-paused:{}:{}:{}",session.as_ref(),request.operation_id.as_ref(),request.expected_pause_revision);
        let id = native_pause::native_key("resume-request",session,&request.idempotency_key)?;
        let ack = required_ack(self.append_event_tx(&mut tx,&native_pause::native_control_event(session,&request.operation_id,
            id,"turn/resume-requested",cause.into(),json!({"request":request,"request_digest":digest_payload(request)?,"owner_digest":digest_payload(owner)?})),None).await?)?;
        tx.commit().await?;
        Ok(NativeResumeRequestReceipt {operation_id:request.operation_id.clone(),request_event_id:ack.event_id,seq:ack.seq,duplicate:false})
    }

    pub async fn native_resume_capacity_available(&self, owner: &PrincipalRef, session: &AgentSessionId,
        request: &NativeResumeRequest) -> Result<bool, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        if !request_is_current_tx(&mut tx,session,request).await? { return Err(SessionStoreError::ExecutionFenced); }
        let ready = nomifun_db::native_resume_capacity_available_tx(
            &mut tx,&owner.principal_id,session.as_ref(),request.operation_id.as_ref()).await.map_err(admission_error)?;
        tx.commit().await?;
        Ok(ready)
    }

    pub async fn pending_native_resume_requests(&self, after_rowid: i64, limit: u32)
        -> Result<Vec<PendingNativeResumeRequest>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let limit=limit.clamp(1,500) as usize;
        let mut cursor=after_rowid;
        let mut pending = Vec::new();
        loop {
        let rows: Vec<(i64,String,String)> = sqlx::query_as(
            "SELECT event.rowid,event.session_id,event.event_id FROM agent_events event \
             JOIN agent_sessions session ON session.agent_session_id=event.session_id \
             JOIN agent_turns turn ON turn.session_id=event.session_id AND turn.operation_id=event.correlation_id \
             JOIN agent_session_heads head ON head.session_id=turn.session_id AND head.active_turn_id=turn.operation_id \
             WHERE event.kind='turn/resume-requested' AND event.rowid>? AND session.state='live' \
               AND turn.state='running' AND turn.native_pause_json IS NOT NULL AND head.status='paused' \
               AND json_extract(turn.native_pause_json,'$.revision')=json_extract(event.inline_json,'$.request.expected_pause_revision') \
               AND turn.native_checkpoint_revision=json_extract(event.inline_json,'$.request.expected_checkpoint_revision') \
               AND turn.native_checkpoint_digest=json_extract(event.inline_json,'$.request.expected_checkpoint_digest') \
               AND NOT EXISTS(SELECT 1 FROM agent_events blocked WHERE blocked.session_id=event.session_id \
                   AND blocked.kind='turn/resume-blocked' AND blocked.causation_event_id=event.event_id) \
             ORDER BY event.rowid LIMIT ?",
        ).bind(cursor).bind(limit as i64).fetch_all(&mut *tx).await?;
        if rows.is_empty() { break; }
        let reached_end=rows.len()<limit;
        cursor=rows.last().expect("non-empty queue page").0;
        for (rowid,session,id) in rows {
            let session = AgentSessionId::from(session);
            let owner = live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref;
            let event = event_by_event_id_tx(&mut tx,&id).await?.map(event_from_row).transpose()?
                .ok_or_else(||SessionStoreError::InvalidEvent("queued resume event missing".into()))?;
            let payload = payload_value_for_event_tx(&mut tx,&event).await?;
            let request: NativeResumeRequest = serde_json::from_value(payload.get("request").cloned()
                .ok_or_else(||SessionStoreError::InvalidEvent("queued resume has no original request".into()))?)?;
            let original = request_record_tx(&mut tx,&session,&owner,&request).await?
                .ok_or_else(||SessionStoreError::InvalidEvent("queued resume identity differs from its original request".into()))?;
            if original.event_id != event.event_id { return Err(SessionStoreError::InvalidEvent("queued resume request identity differs".into())); }
            if !request_is_current_tx(&mut tx,&session,&request).await? { continue; }
            pending.push(PendingNativeResumeRequest {rowid,agent_session_id:session,owner,request:request.clone(),
                receipt:NativeResumeRequestReceipt {operation_id:request.operation_id,request_event_id:event.event_id,seq:event.seq,duplicate:true}});
            if pending.len()==limit { break; }
        }
        // Known retired domain intents do not starve a later live request.
        // This is keyset traversal over the original Event chain, with no
        // separate pending ledger or restart authority.
        if pending.len()==limit || reached_end { break; }
        }
        tx.commit().await?;
        Ok(pending)
    }

    pub async fn block_native_resume_request(&self, owner: &PrincipalRef, session: &AgentSessionId,
        request: &NativeResumeRequest, reason: &str) -> Result<(), SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let Some(event) = request_record_tx(&mut tx,session,owner,request).await? else { return Err(SessionStoreError::ExecutionFenced); };
        if !request_is_current_tx(&mut tx,session,request).await? || blocked_tx(&mut tx,session,&event.event_id).await? { return Ok(()); }
        let reason = reason.chars().take(2048).collect::<String>();
        let id = native_pause::native_key("resume-blocked",session,&request.idempotency_key)?;
        self.append_event_tx(&mut tx,&native_pause::native_control_event(session,&request.operation_id,id,"turn/resume-blocked",event.event_id,
            json!({"request_digest":digest_payload(request)?,"reason":reason})),None).await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(super) async fn queue_state_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
    operation: &OperationId, pause: Option<&NativePauseState>) -> Result<(bool,Option<String>),SessionStoreError> {
    let Some(pause) = pause else { return Ok((false,None)); };
    if !target_is_current_tx(tx,session,operation).await? { return Ok((false,None)); }
    let row: Option<(String,Option<String>)> = sqlx::query_as(
        "SELECT request.event_id,(SELECT json_extract(blocked.inline_json,'$.reason') FROM agent_events blocked \
            WHERE blocked.session_id=request.session_id AND blocked.kind='turn/resume-blocked' \
              AND blocked.causation_event_id=request.event_id ORDER BY blocked.seq DESC LIMIT 1) \
         FROM agent_events request JOIN agent_turns turn ON turn.session_id=request.session_id AND turn.operation_id=request.correlation_id \
         WHERE request.session_id=? AND request.correlation_id=? AND turn.state='running' \
           AND turn.native_checkpoint_revision=json_extract(request.inline_json,'$.request.expected_checkpoint_revision') \
           AND turn.native_checkpoint_digest=json_extract(request.inline_json,'$.request.expected_checkpoint_digest') \
           AND request.kind='turn/resume-requested' AND json_extract(request.inline_json,'$.request.expected_pause_revision')=? \
         ORDER BY request.seq DESC LIMIT 1",
    ).bind(session.as_ref()).bind(operation.as_ref()).bind(as_i64(pause.revision,"queued pause revision")?)
        .fetch_optional(&mut **tx).await?;
    Ok(match row { Some((_,reason)) => (reason.is_none(),reason),None => (false,None) })
}
