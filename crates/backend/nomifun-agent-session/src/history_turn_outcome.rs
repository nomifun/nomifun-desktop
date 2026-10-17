//! Read-only exact-Turn outcome hydration inside the caller's SQLite snapshot.
use super::*;

pub(super) async fn read(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, operation: &OperationId)
    -> Result<crate::CanonicalTurnPublicResult, SessionStoreError> {
    let row=sqlx::query_as::<_,StoredTurnRow>(
        "SELECT turn_id,operation_id,state,started_event_id,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(session.as_ref()).bind(operation.as_ref()).fetch_optional(&mut **tx).await?;
    let Some(row)=row else { return Ok(crate::CanonicalTurnPublicResult {turn_outcome:None,result_text:None,user_input_request:None}); };
    if row.turn_id!=operation.as_ref() || row.operation_id!=operation.as_ref() {return Err(SessionStoreError::InvalidSession("Turn outcome identity differs".into()));}
    let start=event_by_event_id_tx(tx,&row.started_event_id).await?.map(event_from_row).transpose()?
        .ok_or_else(||SessionStoreError::InvalidSession("Turn outcome start missing".into()))?;
    let terminal=match row.terminal_event_id {
        Some(id)=>Some(event_by_event_id_tx(tx,&id).await?.map(event_from_row).transpose()?
            .ok_or_else(||SessionStoreError::InvalidSession("Turn outcome terminal missing".into()))?),None=>None,
    };
    let status=match row.state.as_str() {"accepted"|"running"=>TurnReceiptStatus::Running,
        "completed"=>TurnReceiptStatus::Completed,"failed"=>TurnReceiptStatus::Failed,
        "cancelled"|"interrupted"=>TurnReceiptStatus::Cancelled,
        _=>return Err(SessionStoreError::InvalidSession("Turn outcome state invalid".into()))};
    let head=head_by_id_tx(tx,session.as_ref()).await?;
    let upper=terminal.as_ref().map_or(head.last_seq,|event|event.seq);
    let rows=sqlx::query_as::<_,StoredEventRow>(
        "SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,\
         e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id FROM agent_events e \
         LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
         WHERE e.session_id=? AND e.seq>=? AND e.seq<=? AND (e.kind IN \
         ('turn/started','turn/completed','turn/failed','turn/cancelled','message/content-part','message/completed') \
         OR (e.kind='runtime/progress-recorded' AND COALESCE(json_extract(e.inline_json,'$.event.event'),\
         json_extract(CAST(p.body AS TEXT),'$.value.event.event')) IN \
         ('model_step_started','output_text_delta','completion_delivered','model_output_truncated','model_response_rejected','tool_call_completed','voice_model_step_superseded','user_input_requested','turn_waiting','turn_wait_resolved','steering_inputs','tool_started'))) \
         ORDER BY e.seq LIMIT 1000001")
        .bind(session.as_ref()).bind(as_i64(start.seq,"Turn outcome start")?).bind(as_i64(upper,"Turn outcome end")?)
        .fetch_all(&mut **tx).await?;
    if rows.len()>1_000_000 {return Err(SessionStoreError::InvalidPayload("Turn outcome event budget exceeded".into()));}
    let mut events=Vec::with_capacity(rows.len()); let mut payloads=BTreeMap::new();
    for row in rows {let event=event_from_row(row)?;let value=payload_value_for_event_tx(tx,&event).await?;
        payloads.insert(event.event_id.as_ref().to_owned(),value);events.push(event);}
    let receipt=TurnReceipt {agent_session_id:session.clone(),operation_id:operation.clone(),status,
        started_event:Some(start),terminal_event:terminal};
    crate::turn_outcome::derive_turn_public_result(&receipt,
        matches!(head.status.as_str(),"paused"|"reconciliation")&&head.active_turn_id.as_deref()==Some(operation.as_ref()),&events,&payloads)
}

pub(super) async fn hydrate(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, projections: &mut [MessageProjection])
    -> Result<(), SessionStoreError> {
    for projection in projections.iter_mut().filter(|row|row.presentation_intent=="turn_summary") {
        let operation=projection.projection.get("turn_operation_id").and_then(Value::as_str)
            .ok_or_else(||SessionStoreError::InvalidSession("Turn summary operation missing".into()))?;
        let result=read(tx,session,&OperationId::from(operation)).await?;
        if let Some(request)=result.user_input_request {
            projection.projection["user_input_request"]=serde_json::to_value(request)?;
            projection.message_type=Some("tips".into());
        }
        if let Some(outcome)=result.turn_outcome {
            projection.projection["turn_outcome"]=serde_json::to_value(outcome)?;
            projection.semantic_digest=digest_payload(&projection.projection)?.0;
        }
    }
    Ok(())
}

pub(super) async fn attach_for_page(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,head:u64,history:&mut Vec<MessageProjection>)
    ->Result<(),SessionStoreError> {
    if history.is_empty() {return Ok(());}
    let existing=history.iter().map(|row|row.projection_id.clone()).collect::<BTreeSet<_>>();
    let mut query=sqlx::QueryBuilder::<Sqlite>::new(
        "SELECT session_id,turn_id,operation_id,source_message_id,state,terminal_event_id,started_event_id,accepted_at,started_at,finished_at \
         FROM agent_turns WHERE session_id=");
    query.push_bind(session.as_ref()).push(" AND source_message_id IS NOT NULL AND (");
    {
        let mut conditions=query.separated(" OR ");
        for row in history.iter() {
            conditions.push("(COALESCE(started_at,accepted_at)<=").push_bind_unseparated(as_i64(row.first_seq,"page source sequence")?)
                .push_unseparated(" AND COALESCE(finished_at,").push_bind_unseparated(as_i64(head,"page head")?)
                .push_unseparated(")>=").push_bind_unseparated(as_i64(row.first_seq,"page source sequence")?).push_unseparated(")");
            if row.presentation_intent=="message" && row.projection.get("state").and_then(Value::as_str)==Some("accepted")
                && let Some(id)=row.projection_id.strip_prefix("message:") {
                conditions.push("source_message_id=").push_bind_unseparated(id.to_owned());
            }
        }
    }
    query.push(")");
    for row in query.build_query_as::<StoredTurnHistoryRow>().fetch_all(&mut **tx).await? {
        if let Some(summary)=turn_history_projection_from_row(tx, row).await? && !existing.contains(&summary.projection_id) {history.push(summary);}
    }
    history.sort_by(|left,right|right.first_seq.cmp(&left.first_seq).then_with(||right.projection_id.cmp(&left.projection_id)));
    Ok(())
}
