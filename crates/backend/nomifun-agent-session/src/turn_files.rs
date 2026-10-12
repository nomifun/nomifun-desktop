//! Exact, read-only presentation window addressed by the accepted request identity.
use super::*;

impl AgentSessionStore {
    /// Read all facts of a closed Turn, independently of Message pagination.
    pub async fn turn_file_facts_by_root(
        &self,
        session: &AgentSessionId,
        root: &EventId,
    ) -> Result<Option<ChatCausalityFacts>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session.as_ref()).await?;
        let row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT t.operation_id,t.terminal_event_id FROM agent_turns t \
             JOIN agent_events started ON started.session_id=t.session_id AND started.event_id=t.started_event_id \
             JOIN agent_events input ON input.session_id=t.session_id AND input.event_id=started.causation_event_id \
             WHERE t.session_id=? AND input.event_id=? AND input.kind='message/user-accepted' \
             AND started.kind='turn/started' AND started.correlation_id=t.operation_id",
        )
        .bind(session.as_ref()).bind(root.as_ref()).fetch_optional(&mut *tx).await?
        .ok_or_else(|| SessionStoreError::NotFound(root.as_ref().to_owned()))?;
        if row.1.is_none() { return Ok(None); }
        // Accepted input and closed Turn identities cannot change between the
        // lookup and the normal exact-window read. Deletion still fails closed.
        tx.commit().await?;
        self.turn_output_facts(session, &OperationId::from(row.0)).await.map(Some)
    }
}
