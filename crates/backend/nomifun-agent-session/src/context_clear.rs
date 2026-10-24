//! Owner commands for the canonical context floor. Runtime teardown remains
//! the host's responsibility; no Runtime cache or second ledger owns a clear.
use super::*;
use nomifun_agent_contracts::{ContextClearAdmissionProofV1, ContextClearedPayloadV1};

#[derive(Clone, Debug)]
pub struct ClearSessionContext {
    pub idempotency_key: IdempotencyKey,
    pub reason: String,
}

/// Transient proof captured before host teardown. Settlements may append
/// observations meanwhile, but a binding, input or context change invalidates
/// this boundary. Only the Store can construct it.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextClearBoundary {
    session_id: AgentSessionId,
    binding: AgentBindingValue,
    active_set_generation: u64,
    input_seq: i64,
    context_floor: i64,
}

#[derive(Clone, Debug)]
pub enum ContextClearPreparation {
    Ready(ContextClearBoundary),
    AlreadyCleared(SessionEventAppendResult),
}

impl AgentSessionStore {
    /// Run before any host teardown while holding the Session operation fence.
    /// An exact replay returns its original fact even when a successor Turn is
    /// active, so replay never tears down that successor's Runtime.
    pub async fn preflight_context_clear(
        &self, owner: &PrincipalRef, session: &AgentSessionId, request: &ClearSessionContext,
    ) -> Result<ContextClearPreparation, SessionStoreError> {
        self.prepare_context_clear(owner, session, request, None).await
    }

    /// Revalidate a boundary captured before asynchronous archive work. A
    /// successor input must be rejected before host teardown, not merely at
    /// the final clear transaction after its Runtime was already stopped.
    pub async fn validate_context_clear_boundary(
        &self, owner: &PrincipalRef, session: &AgentSessionId, request: &ClearSessionContext,
        expected: &ContextClearBoundary,
    ) -> Result<ContextClearPreparation, SessionStoreError> {
        self.prepare_context_clear(owner, session, request, Some(expected)).await
    }

    async fn prepare_context_clear(
        &self, owner: &PrincipalRef, session: &AgentSessionId, request: &ClearSessionContext,
        expected: Option<&ContextClearBoundary>,
    ) -> Result<ContextClearPreparation, SessionStoreError> {
        validate_request(request)?;
        let mut tx = self.pool.begin().await?;
        validate_owner_tx(&mut tx, owner, session).await?;
        if let Some(existing) = existing_clear_tx(&mut tx, session, request).await? {
            tx.commit().await?;
            return Ok(ContextClearPreparation::AlreadyCleared(existing));
        }
        let boundary = validate_clear_tx(&mut tx, session, false).await?;
        if expected.is_some_and(|expected| expected != &boundary) {
            return Err(SessionStoreError::Conflict("Session context changed after the archive window was read".into()));
        }
        tx.commit().await?;
        Ok(ContextClearPreparation::Ready(boundary))
    }

    /// Commit one context/cleared fact after the host has joined background
    /// work and proved Runtime cleanup. All persisted guards and the exact
    /// preflight boundary are rechecked under the SQLite write fence.
    pub async fn clear_context(
        &self, owner: &PrincipalRef, session: &AgentSessionId, request: &ClearSessionContext,
        expected: &ContextClearBoundary,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_request(request)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        validate_owner_tx(&mut tx, owner, session).await?;
        if let Some(existing) = existing_clear_tx(&mut tx, session, request).await? {
            db_write_observation.commit(tx).await?;
            return Ok(existing);
        }
        if &validate_clear_tx(&mut tx, session, true).await? != expected {
            return Err(SessionStoreError::Conflict("Session context changed before clear commit".into()));
        }
        let predecessor: String = sqlx::query_scalar(
            "SELECT event_id FROM agent_events WHERE session_id=? \
             AND kind IN ('session/ready','turn/completed','turn/failed','turn/cancelled','context/cleared') \
             ORDER BY seq DESC LIMIT 1",
        ).bind(session.as_ref()).fetch_one(&mut *tx).await?;
        let append = SessionEventAppend {
            agent_session_id: session.clone(),
            event_id: EventId::from(format!("context-cleared:{}", Uuid::now_v7())),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: request.idempotency_key.clone(),
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("context/cleared".into()), kind_version: 1,
                correlation_id: CorrelationId::from(session.as_ref()),
                causation_event_id: Some(EventId::from(predecessor)),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(ContextClearedPayloadV1 {
                    reason: request.reason.clone(), admission: Some(ContextClearAdmissionProofV1 {
                        binding_ref: AgentHandoffBindingRefV1::from(&expected.binding),
                        active_set_generation: expected.active_set_generation,
                    }),
                })?)),
            },
        };
        let result = self.append_event_tx(&mut tx, &append, None).await?;
        db_write_observation.commit(tx).await?;
        Ok(result)
    }

    /// A not-yet-accepted product input may continue an earlier clear only at
    /// that exact canonical boundary. Unlike command replay, continuation must
    /// still prove the current binding, active set and absence of later input.
    /// The host keeps its Session operation fence through subsequent admission.
    pub async fn validate_context_clear_continuation(
        &self, owner: &PrincipalRef, session: &AgentSessionId, request: &ClearSessionContext,
    ) -> Result<(), SessionStoreError> {
        validate_request(request)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        validate_owner_tx(&mut tx, owner, session).await?;
        let original = existing_clear_tx(&mut tx, session, request).await?
            .and_then(|result| result.record)
            .ok_or_else(|| SessionStoreError::Conflict("context continuation has no original clear fact".into()))?;
        let proof = recorded_payload_tx(&mut tx, &original).await?.admission
            .ok_or_else(|| SessionStoreError::Conflict("the original context clear has no exact binding proof for continuation".into()))?;
        let current = validate_clear_tx(&mut tx, session, true).await?;
        let clear_seq = as_i64(original.seq, "context clear sequence")?;
        if current.context_floor != clear_seq || current.input_seq > clear_seq
            || AgentHandoffBindingRefV1::from(&current.binding) != proof.binding_ref
            || current.active_set_generation != proof.active_set_generation
        {
            return Err(SessionStoreError::Conflict("Session context changed after the original clear; continuation is not authorized".into()));
        }
        db_write_observation.commit(tx).await?;
        Ok(())
    }
}

fn validate_request(request: &ClearSessionContext) -> Result<(), SessionStoreError> {
    if request.idempotency_key.as_ref().trim().is_empty()
        || request.idempotency_key.as_ref().trim() != request.idempotency_key.as_ref()
        || request.idempotency_key.as_ref().chars().any(char::is_control)
        || request.idempotency_key.as_ref().len() > 1024
        || request.reason.trim().is_empty() || request.reason.trim() != request.reason
        || request.reason.len() > 256 || request.reason.chars().any(char::is_control)
    {
        return Err(SessionStoreError::InvalidSession("context clear requires a bounded command key and reason".into()));
    }
    Ok(())
}

async fn validate_owner_tx(tx: &mut Transaction<'_, Sqlite>, owner: &PrincipalRef, session: &AgentSessionId)
    -> Result<(), SessionStoreError> {
    validate_principal(owner)?;
    let row = session_row_by_id_tx(tx, session.as_ref()).await?;
    require_owner(&row, owner)?;
    require_live_row(row)?;
    Ok(())
}

async fn existing_clear_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, request: &ClearSessionContext)
    -> Result<Option<SessionEventAppendResult>, SessionStoreError> {
    let Some(row) = event_by_producer_key_tx(tx, "session_api", request.idempotency_key.as_ref()).await? else { return Ok(None); };
    let record = event_from_row(row)?;
    if record.agent_session_id != *session || record.kind.0 != "context/cleared"
        || record.kind_version != 1 || record.correlation_id.as_ref() != session.as_ref()
    {
        return Err(SessionStoreError::IdempotencyConflict("context clear key differs from its original command".into()));
    }
    if recorded_payload_tx(tx, &record).await?.reason != request.reason {
        return Err(SessionStoreError::IdempotencyConflict("context clear key differs from its original command".into()));
    }
    let ack = event_ack(&record);
    Ok(Some(SessionEventAppendResult { cursor: ack.cursor.clone(), record: Some(record), ack: Some(ack), persisted: true, duplicate: true }))
}

async fn recorded_payload_tx(tx: &mut Transaction<'_, Sqlite>, record: &SessionEventRecord)
    -> Result<ContextClearedPayloadV1, SessionStoreError> {
    serde_json::from_value(payload_value_for_event_tx(tx, record).await?)
        .map_err(|error| SessionStoreError::InvalidPayload(format!("context clear fact has an invalid typed payload: {error}")))
}

async fn validate_clear_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, settled: bool)
    -> Result<ContextClearBoundary, SessionStoreError> {
    let live = live_session_by_id_tx(tx, session.as_ref()).await?;
    if live.metadata.purpose != nomifun_agent_contracts::SessionPurpose::Conversation
        || live.remote_binding_provenance.is_some()
    {
        return Err(SessionStoreError::Conflict("context clear requires a local Conversation Session".into()));
    }
    let execution_owned: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND relation IN ('attempt','automation')) \
         OR EXISTS(SELECT 1 FROM conversation_execution_links link \
            JOIN agent_executions execution ON execution.execution_id=link.execution_id \
            WHERE link.conversation_id=? AND link.relation='lead' AND link.active=1 \
              AND execution.user_id=? AND execution.deleted_at IS NULL \
              AND execution.status NOT IN ('completed','completed_with_failures','failed','cancelled'))",
    ).bind(session.as_ref()).bind(session.as_ref()).bind(&live.owner_ref.principal_id)
        .fetch_one(&mut **tx).await?;
    if execution_owned != 0 {
        return Err(SessionStoreError::Conflict("AgentExecution owns this Session context".into()));
    }
    let head = head_by_id_tx(tx, session.as_ref()).await?;
    if head.status != "ready" || head.active_turn_id.is_some() {
        return Err(SessionStoreError::Conflict("context clear requires a ready Session with no active Turn".into()));
    }
    // A damaged/rebuilding projection cannot hide a live native producer or a
    // suspended recovery candidate. Terminal producer identities are already
    // permanently fenced by check_native_lease_tx and remain diagnostic facts.
    let active: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND \
         (state NOT IN ('completed','failed','cancelled') OR native_pause_json IS NOT NULL OR native_pause_requested_json IS NOT NULL))",
    ).bind(session.as_ref()).fetch_one(&mut **tx).await?;
    if active != 0 { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
    let blockers = delete_blockers_tx(tx, session.as_ref()).await?;
    if !blockers.is_empty() {
        return Err(SessionStoreError::Conflict("context clear requires settled effects and resource cleanup".into()));
    }
    if settled {
        storage_relocation::require_native_storage_quiescent_tx(tx, Some(session)).await?;
        let pending_writeback: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_events started WHERE started.session_id=? \
             AND started.kind='turn/knowledge-writeback-started' AND NOT EXISTS \
             (SELECT 1 FROM agent_events settled WHERE settled.session_id=started.session_id \
              AND settled.kind='turn/knowledge-writeback-settled' AND settled.causation_event_id=started.event_id))",
        ).bind(session.as_ref()).fetch_one(&mut **tx).await?;
        if pending_writeback != 0 {
            return Err(SessionStoreError::Conflict("context clear requires joined Knowledge write-back work".into()));
        }
    }
    let (input_seq, context_floor): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(MAX(CASE WHEN kind IN ('message/user-accepted','turn/started') THEN seq END),0), \
         COALESCE(MAX(CASE WHEN kind='context/cleared' THEN seq END),0) FROM agent_events WHERE session_id=?",
    ).bind(session.as_ref()).fetch_one(&mut **tx).await?;
    Ok(ContextClearBoundary { session_id: session.clone(), binding: live.agent_binding,
        active_set_generation: head.active_set_generation, input_seq, context_floor })
}
