//! Owner-scoped read projection for historical tool details. The canonical
//! message projection remains a compact digest; already committed, bounded
//! Runtime observations supply the expandable display when history is read.

use std::collections::{BTreeSet, HashMap};

use nomifun_agent_contracts::{AgentSessionId, SessionPayloadBody};
use nomifun_agent_runtime::{AgentEngineEvent, AgentToolResult};
use nomifun_agent_session::MessageProjection;
use nomifun_common::AppError;
use nomifun_db::SqlitePool;
use serde_json::Value;

#[derive(Default, Debug)]
pub(super) struct HistoricalToolObservation {
    pub runtime_hold: Option<nomifun_agent_contracts::RuntimeToolHoldReason>,
    pub turn_id: Option<String>,
    pub args: Option<Value>,
    pub output: Option<String>,
    pub is_error: Option<bool>,
    pub capability_id: Option<String>,
    pub action_id: Option<String>,
    pub result_seq: Option<u64>,
    pub permission_required: Option<nomifun_engine_core::SystemPermissionRequired>,
}

fn observe_tool_event(event: &Value, event_seq: u64, observations: &mut HashMap<String, HistoricalToolObservation>) {
    // Cancellation can stop the Runtime before ToolCompleted is emitted. The
    // owner still commits its bounded settlement while cleanup is permitted.
    if event.get("event").and_then(Value::as_str) == Some("host_tool_settled") {
        if let Some(result) = event.get("result")
            .and_then(|value| serde_json::from_value::<AgentToolResult>(value.clone()).ok())
            .filter(|result| event.get("call_id").and_then(Value::as_str) == Some(result.call_id.as_ref()))
        {
            let observation = observations.entry(result.call_id.as_ref().to_owned()).or_default();
            observation.output = Some(result.output_text());
            observation.is_error = Some(result.is_error);
            observation.result_seq = Some(event_seq);
            observation.permission_required = nomifun_engine_core::project_system_permission_required(
                observation.capability_id.as_deref(), observation.action_id.as_deref(),
                result.is_error, &result.output_text(),
            );
        }
        return;
    }
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
        AgentEngineEvent::ToolStarted { call_id, capability_id, action_id, .. } => {
            let observation = observations.entry(call_id.as_ref().to_owned()).or_default();
            observation.capability_id = Some(capability_id.as_ref().to_owned());
            observation.action_id = Some(action_id.as_ref().to_owned());
        }
        AgentEngineEvent::ToolCompleted { result, runtime_hold, .. } => {
            let observation = observations
                .entry(result.call_id.as_ref().to_owned())
                .or_default();
            observation.output = Some(result.output_text());
            observation.runtime_hold = runtime_hold;
            observation.is_error = Some(result.is_error);
            observation.result_seq = Some(event_seq);
            observation.permission_required = nomifun_engine_core::project_system_permission_required(
                observation.capability_id.as_deref(), observation.action_id.as_deref(),
                result.is_error, &result.output_text(),
            );
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
        let through_seq = selected.iter().map(|(projection, _)| {
            projection.projection.pointer("/timeline/through_seq").and_then(Value::as_u64)
                .unwrap_or(projection.last_seq)
        }).min().unwrap_or_default();
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT event.seq, event.inline_json, payload.body FROM agent_events event \
             LEFT JOIN agent_payloads payload ON payload.payload_id = event.payload_id \
             WHERE event.session_id = ",
        );
        query.push_bind(session_id.as_ref());
        query.push(" AND event.correlation_id = ");
        query.push_bind(&turn_id);
        query.push(" AND event.kind = 'runtime/progress-recorded' AND event.seq >= ");
        query.push_bind(started_at);
        query.push(" AND event.seq <= ");
        query.push_bind(i64::try_from(through_seq).map_err(|error| AppError::Internal(error.to_string()))?);
        query.push(
            " AND COALESCE( \
             json_extract(event.inline_json, '$.event.call.call_id'), \
             json_extract(event.inline_json, '$.event.result.call_id'), \
             json_extract(event.inline_json, '$.event.call_id'), \
             json_extract(CAST(payload.body AS TEXT), '$.value.event.call.call_id'), \
             json_extract(CAST(payload.body AS TEXT), '$.value.event.result.call_id') \
             , json_extract(CAST(payload.body AS TEXT), '$.value.event.call_id') \
             ) IN (",
        );
        {
            let mut calls = query.separated(", ");
            for call_id in desired_calls {
                calls.push_bind(call_id);
            }
        }
        query.push(") ORDER BY event.seq");
        let rows: Vec<(i64, Option<String>, Option<Vec<u8>>)> = query
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;

        let mut by_call = HashMap::new();
        for (seq, inline, body) in rows {
            let Some(payload) = resolved_progress_payload(inline, body)? else { continue };
            let Some(event) = payload.get("event") else { continue };
            let seq = u64::try_from(seq).map_err(|error| AppError::Internal(error.to_string()))?;
            observe_tool_event(event, seq, &mut by_call);
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
    fn host_settlement_does_not_attach_a_mismatched_call_result() {
        let mut observations = HashMap::new();
        observe_tool_event(&json!({
            "event":"host_tool_settled", "call_id":"expected",
            "result": AgentToolResult::text(ToolCallId::from("different"), "foreign", false)
        }), 7, &mut observations);
        assert!(observations.is_empty());
    }

    #[test]
    fn completed_runtime_events_restore_the_same_expandable_tool_details() {
        let mut observations = HashMap::new();
        observe_tool_event(&json!({
            "event": "tool_call_completed", "step": 1,
            "call": {"call_id": "call-1", "name": "read_file", "arguments": {"path": "src/app.ts"}}
        }), 7, &mut observations);
        observe_tool_event(&json!({
            "event": "tool_completed", "step": 1,
            "result": {"call_id": "call-1", "output": [{"type": "text", "text": "file contents"}], "is_error": false}
        }), 7, &mut observations);
        let observation = observations.get("call-1").unwrap();
        assert_eq!(observation.args.as_ref().unwrap()["path"], "src/app.ts");
        assert_eq!(observation.is_error, Some(false));
        assert_eq!(observation.output.as_deref(), Some("file contents"));
    }

    #[tokio::test]
    async fn cancellation_keeps_the_host_settlement_without_a_runtime_tool_completed_event() {
        let (journal, pool) = test_fixture().await;
        let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        journal.append(json!({
            "event":"tool_call_completed", "step":1,
            "call":{"call_id":"cancel-call","name":"exec_command","arguments":{"command":"bun","args":["wait.mjs"]}}
        }).to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        journal.append(json!({
            "event":"host_tool_dispatch", "dispatch":{
                "operation_id":"cancel-operation", "call_id":"cancel-call",
                "capability_id":"workspace.process", "action_id":"workspace.process/exec", "model_name":"exec_command"
            }
        }).to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        store.cancel_active_turn(&session_id, "cancel-test".into(), "session-api".into()).await.unwrap();
        let output = json!({"state":"cancelled","success":false,"output":{"text":"STARTED"},"cleanup":{"reaped":true}}).to_string();
        journal.append(json!({
            "event":"host_tool_settled", "operation_id":"cancel-operation", "call_id":"cancel-call",
            "result": AgentToolResult::text(ToolCallId::from("cancel-call"), output.clone(), true), "error":null
        }).to_string(), None, EngineJournalWrite::Settlement).await.unwrap();
        let (history, _, _) = store.message_history_before(&session_id, None, 50).await.unwrap();
        let tool = history.iter().find(|item| item.presentation_intent == "tool").unwrap();
        let details = load_historical_tool_observations(&pool, &session_id, &history).await.unwrap();
        let detail = details.get(&tool.projection_id).unwrap();
        assert_eq!(detail.output.as_deref(), Some(output.as_str()));
        assert_eq!(detail.is_error, Some(true));
        assert_eq!(detail.args.as_ref().unwrap()["command"], "bun");
    }

    #[tokio::test]
    async fn cold_history_keeps_rejected_calls_without_owner_dispatch() {
        let (journal, pool) = test_fixture().await;
        let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let call = json!({"event":"tool_call_completed","step":1,
            "call":{"call_id":"rejected-report","name":"report_completion","arguments":{"criteria":[]}}});
        journal.append(call.to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        let output = json!({"status":"not_executed","code":"INVALID_TOOL_ARGUMENTS","tool":"report_completion",
            "issues":[{"instance_path":"/criteria","error":"missing criterion"}],
            "message":"No call in this batch was executed."}).to_string();
        let completed = AgentEngineEvent::ToolCompleted { runtime_hold: None, step:1,
            result:AgentToolResult::text(ToolCallId::from("rejected-report"),output.clone(),true)};
        journal.append(serde_json::to_string(&completed).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
        // An unpaired proposal never becomes an executed or rejected result.
        journal.append(json!({"event":"tool_call_completed","step":2,
            "call":{"call_id":"unfinished","name":"write_file","arguments":{"path":"x","content":"unexecuted"}}}
        ).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
        journal.append(json!({"event":"tool_call_completed","step":3,
            "call":{"call_id":"wrong-step","name":"report_completion","arguments":{}}}
        ).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
        journal.append(serde_json::to_string(&AgentEngineEvent::ToolCompleted { runtime_hold: None, step:4,
            result:AgentToolResult::text(ToolCallId::from("wrong-step"),"foreign step",true)}).unwrap(),
            None,EngineJournalWrite::Progress).await.unwrap();
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let mut other = store.get_live_session(&session_id).await.unwrap();
        other.agent_session_id = uuid::Uuid::now_v7().to_string().into();
        other.next_seq = 1;
        let other_id = other.agent_session_id.clone();
        store.create_session(nomifun_agent_session::CreateSessionRequest::new(other,1,"other-open",
            "session_api".into(),"other-open".into(),"other-open".into())).await.unwrap();
        let before_events:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events").fetch_one(&pool).await.unwrap();
        let before_projections:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_messages").fetch_one(&pool).await.unwrap();
        let (history,_,total) = store.message_history_before(&session_id,None,50).await.unwrap();
        let tool = history.iter().find(|item|item.projection["tool_summary"]["call_id"]=="rejected-report")
            .expect("pre-dispatch failure remains inspectable after cold reload");
        assert_eq!(tool.presentation_intent,"tool");
        assert_eq!(tool.projection["tool_summary"]["name"],"report_completion");
        assert!(history.iter().all(|item|item.projection["tool_summary"]["call_id"]!="unfinished"));
        assert!(history.iter().all(|item|item.projection["tool_summary"]["call_id"]!="wrong-step"));
        let details = load_historical_tool_observations(&pool,&session_id,&history).await.unwrap();
        let detail = details.get(&tool.projection_id).unwrap();
        assert_eq!(detail.args.as_ref().unwrap()["criteria"],json!([]));
        assert_eq!(detail.output.as_deref(),Some(output.as_str()));
        assert_eq!(detail.is_error,Some(true));
        assert!(detail.turn_id.is_some());
        let display_id = tool.projection["correlation_id"].as_str().unwrap();
        let lookup = store.message_by_id(&session_id,display_id).await.unwrap().unwrap();
        assert_eq!(lookup,tool.clone());
        assert!(store.message_by_id(&other_id,display_id).await.unwrap().is_none());
        assert!(store.message_by_id(&session_id,&uuid::Uuid::now_v7().to_string()).await.unwrap().is_none());
        assert!(store.message_by_id(&session_id,"bad-id").await.unwrap().is_none());
        let mut cursor = None;
        let mut paged = Vec::new();
        loop {
            let (page,has_more,page_total) = store.message_history_before(&session_id,cursor,1).await.unwrap();
            assert_eq!(page_total,total);
            assert_eq!(page.len(),1);
            cursor = Some(page[0].first_seq);
            paged.push(page[0].projection_id.clone());
            if !has_more { break; }
        }
        assert_eq!(paged.len(),total as usize);
        assert_eq!(paged.iter().filter(|id|*id==&tool.projection_id).count(),1);
        let (again,_,again_total) = store.message_history_before(&session_id,None,50).await.unwrap();
        assert_eq!(total,again_total);
        assert_eq!(tool.projection_id,again.iter().find(|item|item.projection["tool_summary"]["call_id"]=="rejected-report").unwrap().projection_id);
        assert_eq!(before_events,sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_events").fetch_one(&pool).await.unwrap());
        assert_eq!(before_projections,sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_messages").fetch_one(&pool).await.unwrap());
    }

    #[tokio::test]
    async fn system_permission_cold_history_uses_the_exact_committed_result_and_action() {
        let (journal, pool) = test_fixture().await;
        let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let body = json!({"schema":"system-permission-required.v1","capability":"computer",
            "action_id":"computer/observe","operation":"screenshot","permissions":["screen_recording"],
            "execution":"not_executed"}).to_string();
        let carrier = nomifun_engine_core::system_permission_error_carrier("SYSTEM_PERMISSION_REQUIRED", &body).unwrap();
        let mut exact_results = HashMap::new();
        for (index, (capability, is_error, output)) in [
            ("computer", true, carrier.as_str()), ("computer", false, carrier.as_str()),
            ("browser", true, carrier.as_str()), ("computer", true, body.as_str()),
        ].into_iter().enumerate() {
            let call_id = format!("permission-{index}");
            journal.append(json!({"event":"tool_call_completed","step":1,"call":{
                "call_id":call_id,"name":"computer","arguments":{"action":"screenshot"}
            }}).to_string(), None, EngineJournalWrite::Progress).await.unwrap();
            journal.append(serde_json::to_string(&AgentEngineEvent::ToolStarted {
                step: 1, call_id: call_id.clone().into(), capability_id: capability.into(),
                action_id: "computer/observe".into(),
            }).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();
            // The original owner call projection and native Runtime result
            // share the same canonical chain; no second tool row is created.
            journal.append(json!({"event":"host_tool_dispatch","dispatch":{
                "operation_id":format!("operation-{index}"),"call_id":call_id,
                "capability_id":capability,"action_id":"computer/observe","model_name":"computer"
            }}).to_string(), None, EngineJournalWrite::Progress).await.unwrap();
            journal.append(json!({"event":"host_tool_settled","operation_id":format!("operation-{index}"),
                "call_id":call_id,"result":null,"error":"typed Kernel refusal"
            }).to_string(), None, EngineJournalWrite::Settlement).await.unwrap();
            let completed = AgentEngineEvent::ToolCompleted { runtime_hold: None, step: 1,
                result: AgentToolResult::text(call_id.clone().into(), output, is_error),
            };
            let ack_seq = journal.append_with_receipt(serde_json::to_string(&completed).unwrap(),
                None, EngineJournalWrite::Progress).await.unwrap().expect("canonical append ack");
            let persisted: i64 = sqlx::query_scalar("SELECT seq FROM agent_events WHERE session_id = ? \
                AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_completed' \
                AND json_extract(inline_json,'$.event.result.call_id')=?")
                .bind(session_id.as_ref()).bind(&call_id).fetch_one(&pool).await.unwrap();
            assert_eq!(ack_seq, persisted as u64);
            exact_results.insert(call_id, ack_seq);
        }
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let (history, _, _) = store.message_history_before(&session_id, None, 50).await.unwrap();
        let details = load_historical_tool_observations(&pool, &session_id, &history).await.unwrap();
        for index in 0..4 {
            let call_id = format!("permission-{index}");
            let tools = history.iter().filter(|item| item.projection["tool_summary"]["call_id"] == call_id)
                .collect::<Vec<_>>();
            assert_eq!(tools.len(), 1);
            let detail = details.get(&tools[0].projection_id).unwrap();
            assert_eq!(detail.result_seq, exact_results.get(&call_id).copied());
            assert_eq!(detail.permission_required.is_some(), index == 0);
            assert_eq!(detail.is_error, Some(index != 1));
            assert!(detail.turn_id.is_some());
            assert_eq!(detail.action_id.as_deref(), Some("computer/observe"));
        }
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
        let completed = AgentEngineEvent::ToolCompleted { runtime_hold: None,
            step: 1,
            result: AgentToolResult::text(ToolCallId::from("call-1"), large_output.clone(), false),
        };
        journal.append(serde_json::to_string(&completed).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();

        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let (history, _, _) = store.message_history_before(&session_id, None, 50).await.unwrap();
        let tool = history.iter().find(|projection| projection.presentation_intent == "tool")
            .expect("tool projection survives navigation");
        assert_eq!(history.iter().filter(|projection|projection.presentation_intent=="tool").count(),1,
            "an owner-backed call must not be duplicated by the Runtime history projection");
        let details = load_historical_tool_observations(&pool, &session_id, &history).await.unwrap();
        let detail = details.get(&tool.projection_id).expect("tool detail is rehydrated");
        assert_eq!(detail.args.as_ref().unwrap()["path"], "src/app.ts");
        assert_eq!(detail.output.as_deref(), Some(large_output.as_str()));
        assert_eq!(detail.is_error, Some(false));
        assert!(detail.turn_id.is_some());
    }
}
