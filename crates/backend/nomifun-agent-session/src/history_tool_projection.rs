//! Read-only history for settled Runtime calls without an owner projection.
//! Parameter refusals and internal controls never dispatch an owner, but their
//! paired canonical results must remain inspectable in the conversation.

use nomifun_agent_contracts::{AgentSessionId, canonical_json_bytes, digest_bytes, digest_payload};
use serde_json::json;
use sqlx::{Sqlite, Transaction};
use uuid::Uuid;

use crate::{MessageProjection, SessionStoreError};

// Pair within the exact Session, Turn and model step. An owner-backed call is
// already represented by agent_messages and must not appear a second time.
const HISTORY: &str = r#"
WITH progress AS (
    SELECT event.session_id, event.seq, event.correlation_id,
           COALESCE(json_extract(event.inline_json, '$.event'),
                    json_extract(CAST(payload.body AS TEXT), '$.value.event')) AS observation
    FROM agent_events event
    LEFT JOIN agent_payloads payload ON payload.payload_id = event.payload_id
    WHERE event.session_id = ? AND event.kind = 'runtime/progress-recorded'
      AND event.producer_id = 'runtime_supervisor'
), missing_tools AS (
    SELECT call.session_id, call.seq AS first_seq, result.seq AS last_seq,
           turn.source_message_id, turn.turn_id AS operation_id,
           json_extract(call.observation, '$.call.call_id') AS call_id,
           json_extract(call.observation, '$.call.name') AS name
    FROM progress call
    JOIN progress result ON result.correlation_id = call.correlation_id
      AND json_extract(result.observation, '$.event') = 'tool_completed'
      AND json_extract(result.observation, '$.step') = json_extract(call.observation, '$.step')
      AND json_extract(result.observation, '$.result.call_id') = json_extract(call.observation, '$.call.call_id')
      AND json_type(result.observation, '$.result.is_error') IN ('true', 'false')
      AND result.seq > call.seq
    JOIN agent_turns turn ON turn.session_id = call.session_id AND turn.turn_id = call.correlation_id
    WHERE json_extract(call.observation, '$.event') = 'tool_call_completed'
      AND json_type(call.observation, '$.call.call_id') = 'text'
      AND json_type(call.observation, '$.call.name') = 'text'
      AND json_extract(call.observation, '$.call.call_id') NOT LIKE 'agent-instructions:%'
      AND turn.source_message_id IS NOT NULL
      AND call.seq >= COALESCE(turn.started_at, turn.accepted_at)
      AND (turn.finished_at IS NULL OR result.seq <= turn.finished_at)
      AND NOT EXISTS (
          SELECT 1 FROM agent_messages message
          WHERE message.session_id = call.session_id AND message.presentation_intent = 'tool'
            AND message.first_seq >= COALESCE(turn.started_at, turn.accepted_at)
            AND (turn.finished_at IS NULL OR message.first_seq <= turn.finished_at)
            AND json_extract(message.projection_json, '$.tool_summary.call_id') = json_extract(call.observation, '$.call.call_id')
      )
)
"#;

#[derive(sqlx::FromRow)]
struct ToolHistoryRow {
    session_id: String,
    first_seq: i64,
    last_seq: i64,
    source_message_id: String,
    operation_id: String,
    call_id: String,
    name: String,
}

pub(crate) async fn page(
    tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, before: i64, limit: i64,
) -> Result<(Vec<MessageProjection>, i64), SessionStoreError> {
    let rows = sqlx::query_as::<_, ToolHistoryRow>(&format!(
        "{HISTORY} SELECT * FROM missing_tools WHERE first_seq < ? ORDER BY first_seq DESC, call_id DESC LIMIT ?"
    )).bind(session.as_ref()).bind(before).bind(limit).fetch_all(&mut **tx).await?;
    let total: i64 = sqlx::query_scalar(&format!("{HISTORY} SELECT COUNT(*) FROM missing_tools"))
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    Ok((rows.into_iter().map(project).collect::<Result<_, _>>()?, total))
}

pub(crate) async fn by_message_id(
    tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, message_id: &str,
) -> Result<Option<MessageProjection>, SessionStoreError> {
    let Ok(id) = Uuid::parse_str(message_id) else { return Ok(None) };
    if id.get_version_num() != 7 { return Ok(None); }
    let normalized = id.to_string();
    // The derived UUID preserves the source's timestamp. Restrict the lookup
    // to that exact source timestamp, then compare the full deterministic ID.
    let rows = sqlx::query_as::<_, ToolHistoryRow>(&format!(
        "{HISTORY} SELECT * FROM missing_tools WHERE substr(source_message_id, 1, 13) = ?"
    )).bind(session.as_ref()).bind(&normalized[..13]).fetch_all(&mut **tx).await?;
    for row in rows {
        let projection = project(row)?;
        if projection.projection["correlation_id"] == normalized { return Ok(Some(projection)); }
    }
    Ok(None)
}

fn project(row: ToolHistoryRow) -> Result<MessageProjection, SessionStoreError> {
    let root = Uuid::parse_str(&row.source_message_id)
        .map_err(|_| SessionStoreError::InvalidSession("tool history source is not a UUID".into()))?;
    if root.get_version_num() != 7 || row.first_seq < 0 || row.last_seq < row.first_seq {
        return Err(SessionStoreError::InvalidSession("tool history has an invalid source or sequence".into()));
    }
    // A stable display UUID permits repeated reads and detail expansion. It is
    // derived from the full Turn/call identity, never an execution identity.
    let digest = digest_bytes(&canonical_json_bytes(&json!([
        "runtime-tool-history", row.session_id, row.operation_id, row.call_id
    ]))?).0;
    let mut bytes = *root.as_bytes();
    for (index, byte) in bytes[6..].iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16)
            .expect("digest_bytes returns hexadecimal SHA-256");
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let message_id = Uuid::from_bytes(bytes).to_string();
    let projection_id = format!("runtime-tool:{message_id}");
    let projection = json!({
        "projection_id": projection_id, "correlation_id": message_id,
        "presentation_intent": "tool", "state": "recorded", "turn_id": row.source_message_id,
        "tool_summary": {"call_id": row.call_id, "name": row.name},
    });
    Ok(MessageProjection {
        session_id: row.session_id.into(), projection_id,
        first_seq: row.first_seq as u64, last_seq: row.last_seq as u64,
        presentation_intent: "tool".into(), message_type: None, message_status: None,
        semantic_digest: digest_payload(&projection)?.0, projection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_identity_is_stable_and_scoped_to_the_whole_turn_and_session() {
        let row = |session:&str,turn:&str| ToolHistoryRow {
            session_id:session.into(),operation_id:turn.into(),
            source_message_id:"0190f5fe-7c00-7a00-8000-000000000002".into(),
            call_id:"call-0".into(),name:"report_completion".into(),first_seq:5,last_seq:6,
        };
        let first = project(row("session-a","turn-a")).unwrap();
        assert_eq!(first,project(row("session-a","turn-a")).unwrap());
        assert_ne!(first.projection_id,project(row("session-b","turn-a")).unwrap().projection_id);
        assert_ne!(first.projection_id,project(row("session-a","turn-b")).unwrap().projection_id);
        assert_eq!(Uuid::parse_str(first.projection["correlation_id"].as_str().unwrap()).unwrap().get_version_num(),7);
        assert!(first.projection["tool_summary"].get("capability_id").is_none());
        assert!(first.projection["tool_summary"].get("operation_id").is_none());
    }
}
