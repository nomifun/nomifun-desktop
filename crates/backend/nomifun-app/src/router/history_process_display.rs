//! Owner-scoped read projection for historical tool details. The canonical
//! message projection remains a compact digest; already committed, bounded
//! Runtime observations supply the expandable display when history is read.

use std::collections::{BTreeSet, HashMap};

use nomifun_agent_contracts::{AgentSessionId, SessionPayloadBody};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_agent_session::MessageProjection;
use nomifun_common::AppError;
use nomifun_db::SqlitePool;
use serde_json::Value;

#[derive(Default, Debug)]
pub(super) struct HistoricalToolObservation {
    pub turn_id: Option<String>,
    pub args: Option<Value>,
    pub output: Option<String>,
    pub is_error: Option<bool>,
}

fn observe_tool_event(event: &Value, observations: &mut HashMap<String, HistoricalToolObservation>) {
    let Ok(event) = serde_json::from_value::<AgentEngineEvent>(event.clone()) else {
        return;
    };
    match event {
        AgentEngineEvent::ToolCallCompleted { call, .. } => {
            observations
                .entry(call.call_id.as_ref().to_owned())
                .or_default()
                .args = Some(call.arguments.0);
        }
        AgentEngineEvent::ToolCompleted { result, .. } => {
            let observation = observations
                .entry(result.call_id.as_ref().to_owned())
                .or_default();
            observation.output = Some(result.output_text());
            observation.is_error = Some(result.is_error);
        }
        _ => {}
    }
}

fn resolved_progress_payload(inline: Option<String>, body: Option<Vec<u8>>) -> Result<Option<Value>, AppError> {
    if let Some(inline) = inline {
        return serde_json::from_str(&inline)
            .map(Some)
            .map_err(|error| AppError::Internal(error.to_string()));
    }
    let Some(body) = body else { return Ok(None) };
    let payload: SessionPayloadBody = serde_json::from_slice(&body)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    match payload {
        SessionPayloadBody::Json(value) => Ok(Some(value.0)),
        _ => Ok(None),
    }
}

pub(super) async fn load_historical_tool_observations(
    pool: &SqlitePool,
    session_id: &AgentSessionId,
    projections: &[MessageProjection],
) -> Result<HashMap<String, HistoricalToolObservation>, AppError> {
    let tool_projections = projections
        .iter()
        .filter(|projection| projection.presentation_intent == "tool")
        .filter_map(|projection| {
            let call_id = projection
                .projection
                .get("tool_summary")?
                .get("call_id")?
                .as_str()?;
            Some((projection, call_id))
        })
        .collect::<Vec<_>>();
    if tool_projections.is_empty() {
        return Ok(HashMap::new());
    }
    let min_seq = tool_projections.iter().map(|(projection, _)| projection.first_seq).min().unwrap();
    let max_seq = tool_projections.iter().map(|(projection, _)| projection.last_seq).max().unwrap();
    let min_seq = i64::try_from(min_seq).map_err(|error| AppError::Internal(error.to_string()))?;
    let max_seq = i64::try_from(max_seq).map_err(|error| AppError::Internal(error.to_string()))?;
    let turns: Vec<(String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT turn_id, COALESCE(started_at, accepted_at), finished_at, source_message_id \
         FROM agent_turns WHERE session_id = ? AND COALESCE(started_at, accepted_at) <= ? \
           AND (finished_at IS NULL OR finished_at >= ?)",
    )
    .bind(session_id.as_ref())
    .bind(max_seq)
    .bind(min_seq)
    .fetch_all(pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;

    let mut by_projection = HashMap::new();
    for (turn_id, started_at, finished_at, source_message_id) in turns {
        let selected = tool_projections
            .iter()
            .filter(|(projection, _)| {
                let seq = projection.first_seq as i64;
                seq >= started_at && finished_at.is_none_or(|end| seq <= end)
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        let desired_calls = selected
            .iter()
            .map(|(_, call_id)| *call_id)
            .collect::<BTreeSet<_>>();
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT event.inline_json, payload.body FROM agent_events event \
             LEFT JOIN agent_payloads payload ON payload.payload_id = event.payload_id \
             WHERE event.session_id = ",
        );
        query.push_bind(session_id.as_ref());
        query.push(" AND event.correlation_id = ");
        query.push_bind(&turn_id);
        query.push(" AND event.kind = 'runtime/progress-recorded' AND event.seq >= ");
        query.push_bind(started_at);
        query.push(
            " AND COALESCE( \
             json_extract(event.inline_json, '$.event.call.call_id'), \
             json_extract(event.inline_json, '$.event.result.call_id'), \
             json_extract(CAST(payload.body AS TEXT), '$.value.event.call.call_id'), \
             json_extract(CAST(payload.body AS TEXT), '$.value.event.result.call_id') \
             ) IN (",
        );
        {
            let mut calls = query.separated(", ");
            for call_id in desired_calls {
                calls.push_bind(call_id);
            }
        }
        query.push(") ORDER BY event.seq");
        let rows: Vec<(Option<String>, Option<Vec<u8>>)> = query
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;

        let mut by_call = HashMap::new();
        for (inline, body) in rows {
            let Some(payload) = resolved_progress_payload(inline, body)? else { continue };
            let Some(event) = payload.get("event") else { continue };
            observe_tool_event(event, &mut by_call);
        }
        for (projection, call_id) in selected {
            let mut observation = by_call.remove(*call_id).unwrap_or_default();
            observation.turn_id = source_message_id.clone();
            by_projection.insert(projection.projection_id.clone(), observation);
        }
    }
    Ok(by_projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::engine_journal::{EngineJournalWrite, test_fixture};
    use nomifun_agent_runtime::AgentToolResult;
    use nomifun_agent_session::AgentSessionStore;
    use nomifun_chat_model_broker::ToolCallId;
    use serde_json::json;

    #[test]
    fn completed_runtime_events_restore_the_same_expandable_tool_details() {
        let mut observations = HashMap::new();
        observe_tool_event(&json!({
            "event": "tool_call_completed", "step": 1,
            "call": {"call_id": "call-1", "name": "read_file", "arguments": {"path": "src/app.ts"}}
        }), &mut observations);
        observe_tool_event(&json!({
            "event": "tool_completed", "step": 1,
            "result": {"call_id": "call-1", "output": [{"type": "text", "text": "file contents"}], "is_error": false}
        }), &mut observations);
        let observation = observations.get("call-1").unwrap();
        assert_eq!(observation.args.as_ref().unwrap()["path"], "src/app.ts");
        assert_eq!(observation.is_error, Some(false));
        assert_eq!(observation.output.as_deref(), Some("file contents"));
    }

    #[tokio::test]
    async fn cold_history_recovers_tool_details_from_the_committed_runtime_record() {
        let (journal, pool) = test_fixture().await;
        let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let call = json!({
            "event": "tool_call_completed", "step": 1,
            "call": {"call_id": "call-1", "name": "read_file", "arguments": {"path": "src/app.ts"}}
        });
        journal.append(call.to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        journal.append(json!({
            "event": "host_tool_dispatch",
            "dispatch": {
                "operation_id": "tool-operation", "call_id": "call-1",
                "capability_id": "workspace.files", "action_id": "workspace.files/read",
                "model_name": "read_file"
            }
        }).to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        journal.append(json!({
            "event": "host_tool_settled", "operation_id": "tool-operation",
            "call_id": "call-1", "result": {"call_id": "call-1", "output": "file contents"},
            "error": null
        }).to_string(), None, EngineJournalWrite::Settlement).await.unwrap();
        let large_output = "file contents".repeat(6_000);
        let completed = AgentEngineEvent::ToolCompleted {
            step: 1,
            result: AgentToolResult::text(ToolCallId::from("call-1"), large_output.clone(), false),
        };
        journal.append(serde_json::to_string(&completed).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();

        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let (history, _, _) = store.message_history_before(&session_id, None, 50).await.unwrap();
        let tool = history.iter().find(|projection| projection.presentation_intent == "tool")
            .expect("tool projection survives navigation");
        let details = load_historical_tool_observations(&pool, &session_id, &history).await.unwrap();
        let detail = details.get(&tool.projection_id).expect("tool detail is rehydrated");
        assert_eq!(detail.args.as_ref().unwrap()["path"], "src/app.ts");
        assert_eq!(detail.output.as_deref(), Some(large_output.as_str()));
        assert_eq!(detail.is_error, Some(false));
        assert!(detail.turn_id.is_some());
    }
}
