//! Exact completed-Turn Knowledge observations. This is not an execution queue:
//! pending records are never dispatched or replayed by the Store.
use super::*;
use nomifun_agent_contracts::{
    KnowledgeWritebackNotice, KnowledgeWritebackNoticeStatus, KnowledgeWritebackSettledV1,
    KnowledgeWritebackStartedV1, ResolvedSnapshotRef, SemanticSessionEventDraft,
    TURN_KNOWLEDGE_WRITEBACK_STARTED, TURN_KNOWLEDGE_WRITEBACK_SETTLED,
    validate_knowledge_execution_instance,
};

#[derive(Clone, Debug)]
pub struct KnowledgeWritebackStartRequest {
    pub owner: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub binding_version: u64,
    pub execution_instance_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnowledgeWritebackStartReceipt {
    pub start_event_id: EventId,
    pub event_seq: u64,
    /// Only the initial successful append permits the caller to dispatch.
    pub first: bool,
}

fn identity(session: &AgentSessionId, operation: &OperationId, terminal: bool) -> String {
    format!("knowledge-writeback-{}:{}:{}", if terminal { "settled" } else { "started" }, session.as_ref(), operation.as_ref())
}

fn event(session: &AgentSessionId, operation: &OperationId, terminal: bool, cause: EventId, payload: Value) -> SessionEventAppend {
    let id = identity(session, operation, terminal);
    SessionEventAppend {
        agent_session_id: session.clone(), event_id: id.clone().into(), producer_id: "runtime_supervisor".into(), idempotency_key: id.into(),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind(if terminal { TURN_KNOWLEDGE_WRITEBACK_SETTLED } else { TURN_KNOWLEDGE_WRITEBACK_STARTED }.into()),
            kind_version: 1, correlation_id: operation.as_ref().into(), causation_event_id: Some(cause),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
        },
    }
}

impl AgentSessionStore {
    /// Register an observation before dispatching the process-owned executor.
    /// Only `first` grants that one dispatch; duplicate receipts never replay.
    pub async fn start_knowledge_writeback(&self, request: KnowledgeWritebackStartRequest)
        -> Result<KnowledgeWritebackStartReceipt, SessionStoreError> {
        let payload = KnowledgeWritebackStartedV1 {
            operation_id: request.operation_id.clone(), resolved_snapshot_ref: request.resolved_snapshot_ref,
            binding_version: request.binding_version, execution_instance_id: request.execution_instance_id,
        };
        payload.validate().map_err(SessionStoreError::InvalidPayload)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, request.agent_session_id.as_ref()).await?;
        require_owner(&row, &request.owner)?;
        require_live_row(row)?;
        if let Some(existing) = event_by_event_id_tx(&mut tx, &identity(&request.agent_session_id, &request.operation_id, false)).await? {
            let record = event_from_row(existing)?;
            let original: KnowledgeWritebackStartedV1 = serde_json::from_value(payload_value_for_event_tx(&mut tx, &record).await?)?;
            // A restart cannot use an old dispatch identity, even if the old
            // pending observation has already been marked interrupted.
            if original != payload { return Err(SessionStoreError::IdempotencyConflict("Knowledge write-back start differs from its original frozen input".into())); }
            db_write_observation.commit(tx).await?;
            return Ok(KnowledgeWritebackStartReceipt { start_event_id: record.event_id, event_seq: record.seq, first: false });
        }
        let terminal: Option<String> = sqlx::query_scalar("SELECT terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=? AND state='completed'")
            .bind(request.agent_session_id.as_ref()).bind(request.operation_id.as_ref()).fetch_optional(&mut *tx).await?;
        let terminal = terminal.ok_or_else(|| SessionStoreError::ExecutionFenced)?;
        let append = event(&request.agent_session_id, &request.operation_id, false, terminal.into(), serde_json::to_value(payload)?);
        let result = self.append_event_tx(&mut tx, &append, None).await?;
        let ack = required_ack(result)?;
        db_write_observation.commit(tx).await?;
        Ok(KnowledgeWritebackStartReceipt { start_event_id: ack.event_id, event_seq: ack.seq, first: true })
    }

    /// Settle only the original started observation; this cannot change the
    /// Turn terminal, the active Turn, or authorize a new physical operation.
    pub async fn settle_knowledge_writeback(&self, owner: &PrincipalRef, session: &AgentSessionId,
        start_event_id: &EventId, outcome: KnowledgeWritebackSettledV1) -> Result<SessionEventAppendResult, SessionStoreError> {
        outcome.validate().map_err(SessionStoreError::InvalidPayload)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session.as_ref()).await?;
        require_owner(&row, owner)?;
        let append = event(session, &outcome.operation_id, true, start_event_id.clone(), serde_json::to_value(&outcome)?);
        let result = self.append_event_tx_with_policy(&mut tx, &append, None, AppendSessionStatePolicy::KnowledgeWritebackSettlement).await?;
        db_write_observation.commit(tx).await?;
        Ok(result)
    }

    /// Bootstrap-only reconciliation of observations owned by a dead process.
    /// No model, filesystem or tool operation is performed. An interruption
    /// records unconfirmed outcome rather than inventing a failed disk write.
    pub async fn interrupt_stale_knowledge_writebacks(&self, execution_instance_id: &str) -> Result<usize, SessionStoreError> {
        validate_knowledge_execution_instance(execution_instance_id).map_err(SessionStoreError::InvalidPayload)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let rows = sqlx::query_as::<_, StoredEventRow>(
            "SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
             FROM agent_events e JOIN agent_sessions s ON s.agent_session_id=e.session_id \
             WHERE e.kind='turn/knowledge-writeback-started' AND s.state IN ('live','deleting') \
               AND NOT EXISTS(SELECT 1 FROM agent_events z WHERE z.session_id=e.session_id AND z.kind='turn/knowledge-writeback-settled' AND z.causation_event_id=e.event_id) \
             ORDER BY e.session_id,e.seq",
        ).fetch_all(&mut *tx).await?;
        let mut count = 0;
        for row in rows {
            let started = event_from_row(row)?;
            let input: KnowledgeWritebackStartedV1 = serde_json::from_value(payload_value_for_event_tx(&mut tx, &started).await?)?;
            input.validate().map_err(SessionStoreError::InvalidPayload)?;
            if input.execution_instance_id == execution_instance_id { continue; }
            let mut diagnostic = nomifun_agent_contracts::KnowledgeWritebackDiagnostic::new(nomifun_agent_contracts::KnowledgeWritebackStage::Interrupted);
            diagnostic.code = Some("NOMIFUN_STATE_INCONSISTENT".into());
            diagnostic.detail = Some("The previous process ended before the Knowledge write-back result was confirmed; it was not replayed.".into());
            let outcome = KnowledgeWritebackSettledV1 { operation_id: input.operation_id, status: KnowledgeWritebackNoticeStatus::Interrupted,
                written: vec![], detail: None, diagnostic: Some(diagnostic) };
            let append = event(&started.agent_session_id, &outcome.operation_id, true, started.event_id, serde_json::to_value(&outcome)?);
            self.append_event_tx_with_policy(&mut tx, &append, None, AppendSessionStatePolicy::KnowledgeWritebackSettlement).await?;
            count += 1;
        }
        db_write_observation.commit(tx).await?;
        Ok(count)
    }

    /// Read one exact original Turn's renderer summary, including later
    /// Knowledge observations, without modifying either fact or projection.
    pub async fn turn_history_for_operation(&self, owner: &PrincipalRef, session: &AgentSessionId, operation: &OperationId)
        -> Result<Option<MessageProjection>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let row = session_row_by_id_tx(&mut tx, session.as_ref()).await?;
        require_owner(&row, owner)?; require_live_row(row)?;
        let row = sqlx::query_as::<_, StoredTurnHistoryRow>(
            "SELECT session_id,turn_id,operation_id,source_message_id,state,terminal_event_id,started_event_id,accepted_at,started_at,finished_at \
             FROM agent_turns WHERE session_id=? AND operation_id=? AND source_message_id IS NOT NULL",
        ).bind(session.as_ref()).bind(operation.as_ref()).fetch_optional(&mut *tx).await?;
        let mut summaries = match row { Some(row) => turn_history_projection_from_row(&mut tx, row).await?.into_iter().collect::<Vec<_>>(), None => Vec::new() };
        history_turn_outcome::hydrate(&mut tx, session, &mut summaries).await?;
        hydrate(&mut tx, session, &mut summaries).await?;
        tx.commit().await?;
        Ok(summaries.pop())
    }
}

pub(super) fn is_event(kind: &str) -> bool {
    matches!(kind, TURN_KNOWLEDGE_WRITEBACK_STARTED | TURN_KNOWLEDGE_WRITEBACK_SETTLED)
}

pub(super) async fn validate_event_tx(tx: &mut Transaction<'_, Sqlite>, record: &SessionEventRecord, value: &Value) -> Result<(), SessionStoreError> {
    if !is_event(&record.kind.0) { return Ok(()); }
    if record.producer_id.as_ref() != "runtime_supervisor" { return Err(SessionStoreError::ExecutionFenced); }
    let cause = record.causation_event_id.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
    if record.kind.0 == TURN_KNOWLEDGE_WRITEBACK_STARTED {
        let input: KnowledgeWritebackStartedV1 = serde_json::from_value(value.clone())?;
        input.validate().map_err(SessionStoreError::InvalidPayload)?;
        let current = live_session_by_id_tx(tx, record.agent_session_id.as_ref()).await?;
        if current.owner_ref.principal_kind != "user" || current.agent_binding.binding_version != input.binding_version
            || current.agent_binding.resolved_snapshot_ref != input.resolved_snapshot_ref || record.correlation_id.as_ref() != input.operation_id.as_ref() {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let (started, terminal): (String, String) = sqlx::query_as(
            "SELECT started_event_id,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=? AND state='completed'",
        ).bind(record.agent_session_id.as_ref()).bind(input.operation_id.as_ref()).fetch_optional(&mut **tx).await?.ok_or(SessionStoreError::ExecutionFenced)?;
        let terminal = event_by_event_id_tx(tx, &terminal).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let started = event_by_event_id_tx(tx, &started).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let started_payload = payload_value_for_event_tx(tx, &started).await?;
        let snapshot = started_payload.get("resolved_snapshot_ref").or_else(|| started_payload.pointer("/admission/resolved_snapshot_ref"));
        let floor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(record.agent_session_id.as_ref()).fetch_one(&mut **tx).await?;
        let root = started.causation_event_id.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
        let root = event_by_event_id_tx(tx, root.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let root_payload = payload_value_for_event_tx(tx, &root).await?;
        if started.kind.0 != "turn/started" || started.agent_session_id != record.agent_session_id || started.correlation_id != record.correlation_id
            || cause != &terminal.event_id || terminal.kind.0 != "turn/completed" || terminal.agent_session_id != record.agent_session_id
            || terminal.correlation_id != record.correlation_id || snapshot != Some(&serde_json::to_value(&input.resolved_snapshot_ref)?)
            || started.seq <= as_u64(floor,"Knowledge context floor")? || root.kind.0 != "message/user-accepted"
            || root.agent_session_id != record.agent_session_id || root_payload.get("origin").and_then(Value::as_str).is_some_and(|origin| !origin.is_empty()) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let other: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/knowledge-writeback-started'")
            .bind(record.agent_session_id.as_ref()).bind(record.correlation_id.as_ref()).fetch_one(&mut **tx).await?;
        if other != 1 { return Err(SessionStoreError::ExecutionFenced); }
    } else {
        let outcome: KnowledgeWritebackSettledV1 = serde_json::from_value(value.clone())?;
        outcome.validate().map_err(SessionStoreError::InvalidPayload)?;
        let start = event_by_event_id_tx(tx, cause.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let input: KnowledgeWritebackStartedV1 = serde_json::from_value(payload_value_for_event_tx(tx,&start).await?)?;
        if start.kind.0 != TURN_KNOWLEDGE_WRITEBACK_STARTED || start.agent_session_id != record.agent_session_id
            || start.correlation_id != record.correlation_id || input.operation_id != outcome.operation_id
            || outcome.operation_id.as_ref() != record.correlation_id.as_ref() {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/knowledge-writeback-settled'")
            .bind(record.agent_session_id.as_ref()).bind(record.correlation_id.as_ref()).fetch_one(&mut **tx).await?;
        if count != 1 { return Err(SessionStoreError::ExecutionFenced); }
    }
    Ok(())
}

pub(super) async fn hydrate(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, summaries: &mut [MessageProjection]) -> Result<(), SessionStoreError> {
    let operations = summaries.iter().filter(|item| item.presentation_intent == "turn_summary")
        .filter_map(|item| item.projection.get("turn_operation_id").and_then(Value::as_str)).map(str::to_owned).collect::<Vec<_>>();
    if operations.is_empty() { return Ok(()); }
    let mut query = sqlx::QueryBuilder::<Sqlite>::new(
        "SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id \
         FROM agent_events WHERE session_id=",
    );
    query.push_bind(session.as_ref()).push(" AND kind IN ('turn/knowledge-writeback-started','turn/knowledge-writeback-settled') AND correlation_id IN (");
    let mut values = query.separated(","); for operation in &operations { values.push_bind(operation); } values.push_unseparated(") ORDER BY seq");
    let rows = query.build_query_as::<StoredEventRow>().fetch_all(&mut **tx).await?;
    let mut notices = BTreeMap::<String, (EventId, KnowledgeWritebackNotice)>::new();
    for row in rows {
        let record = event_from_row(row)?;
        let value = payload_value_for_event_tx(tx,&record).await?;
        if record.kind.0 == TURN_KNOWLEDGE_WRITEBACK_STARTED {
            let input: KnowledgeWritebackStartedV1 = serde_json::from_value(value)?;
            input.validate().map_err(SessionStoreError::InvalidPayload)?;
            if input.operation_id.as_ref() != record.correlation_id.as_ref() || notices.contains_key(record.correlation_id.as_ref()) {
                return Err(SessionStoreError::InvalidEvent("Knowledge started observation identity is inconsistent".into()));
            }
            notices.insert(input.operation_id.as_ref().into(), (record.event_id, KnowledgeWritebackNotice {
                status: KnowledgeWritebackNoticeStatus::Running, written: vec![], detail: None, diagnostic: None, event_seq: Some(record.seq),
            }));
        } else {
            let outcome: KnowledgeWritebackSettledV1 = serde_json::from_value(value)?;
            outcome.validate().map_err(SessionStoreError::InvalidPayload)?;
            let prior = notices.get_mut(record.correlation_id.as_ref()).ok_or_else(|| SessionStoreError::InvalidEvent("Knowledge settlement has no original start".into()))?;
            if record.causation_event_id.as_ref() != Some(&prior.0) || prior.1.status != KnowledgeWritebackNoticeStatus::Running
                || outcome.operation_id.as_ref() != record.correlation_id.as_ref() {
                return Err(SessionStoreError::InvalidEvent("Knowledge settlement changed its original Turn or was repeated".into()));
            }
            prior.1 = outcome.notice(record.seq);
        }
    }
    for summary in summaries {
        if let Some((_, notice)) = summary.projection.get("turn_operation_id").and_then(Value::as_str).and_then(|operation| notices.get(operation)) {
            summary.projection["knowledge_writeback"] = serde_json::to_value(notice)?;
            summary.semantic_digest = digest_payload(&summary.projection)?.0;
        }
    }
    Ok(())
}
