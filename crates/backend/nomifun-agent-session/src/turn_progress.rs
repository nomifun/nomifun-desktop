//! Bounded exact-Turn observation over the original canonical facts.
use super::*;

async fn latest_boundary_tx(
    tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, operation: &OperationId,
    start: u64, through: u64, boundary: &str,
) -> Result<Option<SessionEventRecord>, SessionStoreError> {
    let row = sqlx::query_as::<_, StoredEventRow>(
        "SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version, \
         e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id FROM agent_events e \
         LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
         WHERE e.session_id=? AND e.correlation_id=? AND e.seq>? AND e.seq<=? AND e.kind='runtime/progress-recorded' \
           AND (COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event')) \
             IN (?,'turn_wait_resolved','model_step_started','tool_started') \
             OR (COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='steering_inputs' \
               AND COALESCE(json_array_length(e.inline_json,'$.event.inputs'),json_array_length(CAST(p.body AS TEXT),'$.value.event.inputs'),0)>0)) \
         ORDER BY e.seq DESC LIMIT 1",
    ).bind(session.as_ref()).bind(operation.as_ref()).bind(as_i64(start, "progress start")?)
        .bind(as_i64(through, "progress cursor")?).bind(boundary).fetch_optional(&mut **tx).await?;
    row.map(event_from_row).transpose()
}

impl AgentSessionStore {
    /// Read lifecycle and the current suspension only. The latest waiting
    /// boundary and its applicable question/reset are at most two payloads;
    /// the shared reducer validates the same typed waiting contract as history.
    pub async fn read_turn_progress(
        &self, session: &AgentSessionId, operation: &OperationId,
    ) -> Result<TurnProgressObservation, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session.as_ref()).await?;
        let row: Option<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT turn_id,state,started_event_id,terminal_event_id,native_pause_json \
             FROM agent_turns WHERE session_id=? AND operation_id=?",
        ).bind(session.as_ref()).bind(operation.as_ref()).fetch_optional(&mut *tx).await?;
        let Some((identity, state, started, terminal, native_pause)) = row else {
            tx.commit().await?;
            return Ok(TurnProgressObservation { status: TurnReceiptStatus::NotFound, cursor: head.last_seq, suspended: false });
        };
        if identity != operation.as_ref() { return Err(SessionStoreError::InvalidSession("Turn progress identity differs".into())); }
        let start = event_by_event_id_tx(&mut tx, &started).await?.map(event_from_row).transpose()?
            .ok_or_else(|| SessionStoreError::InvalidSession("Turn progress start missing".into()))?;
        if start.agent_session_id != *session || start.correlation_id.as_ref() != operation.as_ref()
            || start.kind.0 != "turn/started" || start.kind_version != 1 {
            return Err(SessionStoreError::InvalidEvent("Turn progress start differs".into()));
        }
        let status = match state.as_str() {
            "accepted" | "running" => TurnReceiptStatus::Running,
            "completed" => TurnReceiptStatus::Completed,
            "failed" => TurnReceiptStatus::Failed,
            "cancelled" | "interrupted" => TurnReceiptStatus::Cancelled,
            _ => return Err(SessionStoreError::InvalidSession("Turn progress state invalid".into())),
        };
        let mut suspended = false;
        if status == TurnReceiptStatus::Running {
            if terminal.is_some() { return Err(SessionStoreError::InvalidSession("running Turn progress has a terminal".into())); }
            let native_paused = native_pause.map(|value| serde_json::from_str::<NativePauseState>(&value)).transpose()?.is_some();
            suspended = native_paused || (head.active_turn_id.as_deref() == Some(operation.as_ref())
                && matches!(head.status.as_str(), "paused" | "reconciliation"));
            if !suspended && let Some(boundary) = latest_boundary_tx(&mut tx, session, operation, start.seq, head.last_seq, "turn_waiting").await? {
                let value = payload_value_for_event_tx(&mut tx, &boundary).await?;
                if value.pointer("/event/event").and_then(Value::as_str) == Some("turn_waiting") {
                    let mut events = Vec::with_capacity(2);
                    let mut payloads = BTreeMap::new();
                    if let Some(context) = latest_boundary_tx(&mut tx, session, operation, start.seq, boundary.seq - 1, "user_input_requested").await? {
                        payloads.insert(context.event_id.as_ref().to_owned(), payload_value_for_event_tx(&mut tx, &context).await?);
                        events.push(context);
                    }
                    payloads.insert(boundary.event_id.as_ref().to_owned(), value);
                    events.push(boundary);
                    suspended = crate::turn_outcome::canonical_turn_waiting_outcome(&events, &payloads, operation, head.last_seq)?.is_some();
                }
            }
        } else {
            let id = terminal.ok_or_else(|| SessionStoreError::InvalidSession("terminal Turn progress has no terminal".into()))?;
            let terminal = event_by_event_id_tx(&mut tx, &id).await?.map(event_from_row).transpose()?
                .ok_or_else(|| SessionStoreError::InvalidSession("Turn progress terminal missing".into()))?;
            let expected = match status { TurnReceiptStatus::Completed => "turn/completed", TurnReceiptStatus::Failed => "turn/failed", _ => "turn/cancelled" };
            if terminal.agent_session_id != *session || terminal.correlation_id.as_ref() != operation.as_ref()
                || terminal.kind.0 != expected || terminal.kind_version != 1 || terminal.seq <= start.seq {
                return Err(SessionStoreError::InvalidEvent("Turn progress terminal differs".into()));
            }
        }
        tx.commit().await?;
        Ok(TurnProgressObservation { status, cursor: head.last_seq, suspended })
    }
}
