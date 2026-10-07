//! Latest runtime state reads from the canonical journal; no second state store.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeStateObservation {
    pub sequence: u64,
    pub turn_id: Option<String>,
    pub turn_status: Option<String>,
    pub event: Option<Value>,
}

impl AgentSessionStore {
    /// Read one named runtime fact and its owning lifecycle in the same SQLite
    /// snapshot. Only the latest started turn participates, even if that turn
    /// has not produced this fact. Unrelated later records and message paging
    /// cannot evict the state. The Store does not interpret the engine payload.
    pub async fn latest_runtime_state(
        &self,
        session: &AgentSessionId,
        event_name: &str,
    ) -> Result<RuntimeStateObservation, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session.as_ref()).await?;
        let turn: Option<(String, Option<String>, String, i64)> = sqlx::query_as(
            "SELECT t.operation_id, t.source_message_id, t.state, started.seq \
             FROM agent_turns t JOIN agent_events started ON started.event_id = t.started_event_id \
             WHERE t.session_id = ? ORDER BY started.seq DESC LIMIT 1",
        ).bind(session.as_ref()).fetch_optional(&mut *tx).await?;
        let mut result = RuntimeStateObservation {
            sequence: head.last_seq, turn_id: None, turn_status: None, event: None,
        };
        if let Some((operation, source, status, started_seq)) = turn {
            result.turn_id = source;
            // A native pause suspends a running Turn without inventing a new
            // terminal state. Project the same active operation's canonical
            // head disposition; a terminal Turn always keeps its own state.
            result.turn_status = Some(if status == "running"
                && head.active_turn_id.as_deref() == Some(operation.as_str())
                && head.status == "paused"
            { "paused".to_owned() } else { status });
            let body: Option<String> = sqlx::query_scalar(
                "WITH progress AS (SELECT e.seq, \
                   COALESCE(json_extract(e.inline_json, '$.event'), \
                     json_extract(CAST(p.body AS TEXT), '$.value.event')) AS event \
                 FROM agent_events e \
                 LEFT JOIN agent_payloads p ON p.payload_id = e.payload_id AND p.session_id = e.session_id \
                 WHERE e.session_id = ? AND e.correlation_id = ? AND e.seq > ? \
                   AND e.kind = 'runtime/progress-recorded' \
                   AND e.producer_id = 'runtime_supervisor') \
                 SELECT event FROM progress WHERE json_extract(event, '$.event') = ? \
                 ORDER BY seq DESC LIMIT 1",
            ).bind(session.as_ref()).bind(operation).bind(started_seq).bind(event_name)
                .fetch_optional(&mut *tx).await?;
            if let Some(body) = body {
                result.event = Some(serde_json::from_str(&body)?);
            }
        }
        tx.commit().await?;
        Ok(result)
    }
}
