//! Transition references come from their exact canonical event and Payload.
//! Persisted presentation diagnostics never become fields of a typed payload.

use super::*;

pub(super) async fn hydrate(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &AgentSessionId,
    projections: &mut [MessageProjection],
) -> Result<(), SessionStoreError> {
    let indices: Vec<usize> = projections.iter().enumerate()
        .filter_map(|(index, projection)| (projection.presentation_intent == "agent_transition").then_some(index))
        .collect();
    // Forward delta reads can contain more rows than a history page. Bound each
    // fact lookup rather than constructing an unbounded SQL parameter list.
    for indices in indices.chunks(MAX_EVENT_PAGE_SIZE as usize) {
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM agent_events WHERE session_id = ",
        );
        query.push_bind(session_id.as_ref()).push(" AND seq IN (");
        let mut sequences = query.separated(", ");
        for &index in indices {
            let projection = &projections[index];
            if projection.session_id != *session_id || projection.first_seq != projection.last_seq {
                return Err(invalid("has an invalid Session or event boundary"));
            }
            if digest_payload(&projection.projection)?.0 != projection.semantic_digest {
                return Err(invalid("has an invalid semantic digest"));
            }
            sequences.push_bind(as_i64(projection.first_seq, "transition sequence")?);
        }
        sequences.push_unseparated(")");
        let events: BTreeMap<u64, SessionEventRecord> = query.build_query_as::<StoredEventRow>()
            .fetch_all(&mut **tx).await?.into_iter()
            .map(|row| event_from_row(row).map(|event| (event.seq, event)))
            .collect::<Result<_, _>>()?;
        for &index in indices {
            let projection = &projections[index];
            let event = events.get(&projection.first_seq)
                .ok_or_else(|| invalid("has no canonical event"))?;
            if event.kind.0.as_str() != "session/agent-binding-changed" || event.kind_version != 1 {
                return Err(invalid("does not reference a current Agent transition event"));
            }
            let payload = payload_value_for_event_tx(tx, event).await?;
            let rebuilt = reduce_agent_messages(None, event, &payload)?;
            if rebuilt.session_id != projection.session_id
                || rebuilt.projection_id != projection.projection_id
                || rebuilt.first_seq != projection.first_seq
                || rebuilt.last_seq != projection.last_seq
                || rebuilt.presentation_intent != projection.presentation_intent
                || ["projection_id", "correlation_id", "presentation_intent"].iter()
                    .any(|key| projection.projection.get(*key) != rebuilt.projection.get(*key))
            {
                return Err(invalid("identity differs from its canonical event"));
            }
            projections[index] = rebuilt;
        }
    }
    Ok(())
}

fn invalid(message: &str) -> SessionStoreError {
    SessionStoreError::InvalidEvent(format!("Agent transition projection {message}"))
}
