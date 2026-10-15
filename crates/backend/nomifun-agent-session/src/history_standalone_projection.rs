//! Display provenance for a formally projected assistant notice/synthesis.
//! The marker is derived from the exact typed event, never from a missing Turn.
use super::*;

pub(super) async fn hydrate(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,rows:&mut [MessageProjection])
    ->Result<(),SessionStoreError> {
    for row in rows.iter_mut().filter(|row|row.presentation_intent=="message") {
        if row.projection.get("standalone_projection").is_some() {
            return Err(SessionStoreError::InvalidEvent("stored message cannot author derived standalone provenance".into()));
        }
        let event=sqlx::query_as::<_,StoredEventRow>(
            "SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id \
             FROM agent_events WHERE session_id=? AND seq=? AND kind='message/assistant-projected'")
            .bind(session.as_ref()).bind(as_i64(row.last_seq,"standalone message source")?)
            .fetch_optional(&mut **tx).await?;
        let Some(event)=event else {continue;};
        let event=event_from_row(event)?;
        let payload=payload_value_for_event_tx(tx,&event).await?;
        let canonical=reduce_agent_messages(None,&event,&payload)?;
        if event.kind_version!=1 || row.session_id!=*session || canonical.projection_id!=row.projection_id
            || canonical.first_seq!=row.first_seq || canonical.last_seq!=row.last_seq
            || canonical.semantic_digest!=row.semantic_digest || canonical.projection!=row.projection {
            return Err(SessionStoreError::InvalidEvent("standalone assistant projection differs from its exact canonical source".into()));
        }
        row.projection["standalone_projection"]=json!(true);
        row.semantic_digest=digest_payload(&row.projection)?.0;
    }
    Ok(())
}
