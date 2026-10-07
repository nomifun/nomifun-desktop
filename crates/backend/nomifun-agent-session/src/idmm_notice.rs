//! Display-only IDMM facts share the canonical event and projection transaction.

use nomifun_agent_contracts::{IdmmDecisionNotice, IdmmQuestionRef};

use super::*;

impl AgentSessionStore {
    /// Record a bounded supervisor notice only while its exact assistant question
    /// remains unanswered. A replay returns the first notice even after progress.
    pub async fn append_idmm_notice(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        key: &str,
        notice: IdmmDecisionNotice,
    ) -> Result<Option<SessionEventAppendResult>, SessionStoreError> {
        notice.validate().map_err(|error| SessionStoreError::InvalidPayload(error.into()))?;
        if key.trim().is_empty() || key.trim() != key || key.len() > 512 {
            return Err(SessionStoreError::InvalidPayload("IDMM notice key is invalid".into()));
        }
        let producer = EventProducerId::from("session_api");
        let key = IdempotencyKey::from(format!("idmm-notice:{}:{key}", session_id.as_ref()));
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        require_live_row(row)?;

        if let Some(existing) = event_by_producer_key_tx(&mut tx, producer.as_ref(), key.as_ref()).await? {
            let record = event_from_row(existing)?;
            if record.agent_session_id != *session_id || record.kind.0 != "idmm/notice-recorded" {
                return Err(SessionStoreError::IdempotencyConflict("IDMM notice key belongs to a different fact".into()));
            }
            let persisted: IdmmDecisionNotice = serde_json::from_value(payload_value_for_event_tx(&mut tx, &record).await?)?;
            persisted.validate().map_err(|error| SessionStoreError::InvalidPayload(error.into()))?;
            if persisted.decision.question != notice.decision.question || persisted.status != notice.status {
                return Err(SessionStoreError::IdempotencyConflict("IDMM notice replay changed its question or status".into()));
            }
            let ack = event_ack(&record);
            tx.commit().await?;
            return Ok(Some(SessionEventAppendResult {
                cursor: ack.cursor.clone(), record: Some(record), ack: Some(ack),
                persisted: true, duplicate: true,
            }));
        }

        let question = notice.decision.question.as_ref().expect("validated notice question");
        if !question_is_current_tx(&mut tx, session_id, question).await? {
            tx.commit().await?;
            return Ok(None);
        }
        let cause: String = sqlx::query_scalar(
            "SELECT event_id FROM agent_events WHERE session_id = ? AND seq = ?",
        )
        .bind(session_id.as_ref()).bind(as_i64(question.sequence, "IDMM question sequence")?)
        .fetch_one(&mut *tx).await?;
        let identity = notice.decision.intervention_id.clone();
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(), event_id: EventId::from(identity.clone()),
            producer_id: producer, idempotency_key: key,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("idmm/notice-recorded".into()), kind_version: 1,
                correlation_id: CorrelationId::from(identity), causation_event_id: Some(EventId::from(cause)),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(notice)?)),
            },
        };
        let result = self.append_event_tx(&mut tx, &append, None).await?;
        tx.commit().await?;
        Ok(Some(result))
    }
}

/// Only presentation metadata is compared here. Runtime context still reads
/// canonical events, and notices never modify this source message projection.
pub(super) async fn question_is_current_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &AgentSessionId,
    question: &IdmmQuestionRef,
) -> Result<bool, SessionStoreError> {
    question.validate().map_err(|error| SessionStoreError::InvalidPayload(error.into()))?;
    let head = head_by_id_tx(tx, session_id.as_ref()).await?;
    if head.status != "ready" || head.active_turn_id.is_some() {
        return Ok(false);
    }
    let boundary_changed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id = ? AND seq > ? \
         AND kind IN ('context/cleared', 'session/agent-binding-changed'))",
    )
    .bind(session_id.as_ref())
    .bind(as_i64(question.sequence, "IDMM question sequence")?)
    .fetch_one(&mut **tx)
    .await?;
    if boundary_changed {
        return Ok(false);
    }
    let latest: Option<(String, i64, String, String)> = sqlx::query_as(
        "SELECT projection_id, last_seq, semantic_digest, projection_json FROM agent_messages \
         WHERE session_id = ? AND presentation_intent = 'message' ORDER BY last_seq DESC LIMIT 1",
    ).bind(session_id.as_ref()).fetch_optional(&mut **tx).await?;
    let Some((projection_id, sequence, fingerprint, document)) = latest else { return Ok(false); };
    let completed_source: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id = ? AND seq = ? \
         AND correlation_id = ? AND kind IN ('message/completed', 'message/assistant-projected'))",
    )
    .bind(session_id.as_ref())
    .bind(as_i64(question.sequence, "IDMM question sequence")?)
    .bind(&question.message_id)
    .fetch_one(&mut **tx)
    .await?;
    if !completed_source {
        return Ok(false);
    }
    let document: Value = serde_json::from_str(&document)?;
    Ok(projection_id == format!("message:{}", question.message_id)
        && as_u64(sequence, "IDMM question sequence")? == question.sequence
        && fingerprint == question.fingerprint
        && document.get("state").and_then(Value::as_str) == Some("completed")
        && document.get("correlation_id").and_then(Value::as_str) == Some(question.message_id.as_str()))
}
