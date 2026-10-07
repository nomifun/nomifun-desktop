//! Read-only reasoning lifecycle from the exact canonical Turn and its typed
//! Runtime events. Persisted content alone is never a phase completion proof.

use std::collections::BTreeSet;

use nomifun_agent_contracts::{AgentSessionId, SessionPayloadBody, digest_payload};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_agent_session::MessageProjection;
use nomifun_common::AppError;
use nomifun_db::SqlitePool;
use serde_json::{Value, json};

pub(super) async fn hydrate_thinking_lifecycle(
    pool: &SqlitePool,
    session_id: &AgentSessionId,
    projections: &mut [MessageProjection],
) -> Result<(), AppError> {
    let roots = projections
        .iter()
        .filter(|projection| projection.presentation_intent == "thinking")
        .filter_map(|projection| {
            projection.projection.get("turn_id").and_then(Value::as_str)
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if roots.is_empty() {
        return Ok(());
    }

    // Read the Turn state and latest phase transition in one snapshot. This
    // also covers owner terminals that have no final assistant text.
    let mut tx = pool.begin().await.map_err(internal)?;
    for root in roots {
        let turn: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT turn_id, state, COALESCE(started_at, accepted_at) FROM agent_turns \
             WHERE session_id = ? AND source_message_id = ?",
        )
        .bind(session_id.as_ref())
        .bind(&root)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?;
        let mut active_message_id = None;
        if let Some((operation, state, started_seq)) = turn {
            if matches!(state.as_str(), "accepted" | "running") {
                // Only lifecycle transitions are loaded, rather than every
                // tool result or text body in a potentially long Turn.
                let latest: Option<(Option<String>, Option<Vec<u8>>)> = sqlx::query_as(
                    "SELECT event.inline_json, payload.body FROM agent_events event \
                     LEFT JOIN agent_payloads payload ON payload.payload_id = event.payload_id \
                     WHERE event.session_id = ? AND event.correlation_id = ? \
                       AND event.kind = 'runtime/progress-recorded' AND event.producer_id = 'runtime_supervisor' \
                       AND event.seq >= ? \
                       AND COALESCE(json_extract(event.inline_json, '$.event.event'), \
                           json_extract(CAST(payload.body AS TEXT), '$.value.event.event')) IN ( \
                         'reasoning_delta', 'model_step_started', 'execution_resumed', 'output_text_delta', \
                         'completion_delivered', 'tool_call_delta', 'tool_call_completed', 'tool_started', \
                         'model_output_truncated', 'model_response_rejected', 'delivery_review_superseded', \
                         'turn_completed', 'turn_cancelled', 'turn_paused', 'turn_failed') \
                     ORDER BY event.seq DESC LIMIT 1",
                )
                .bind(session_id.as_ref())
                .bind(operation)
                .bind(started_seq)
                .fetch_optional(&mut *tx)
                .await
                .map_err(internal)?;
                if let Some((inline, body)) = latest {
                    let payload = match (inline, body) {
                        (Some(inline), _) => serde_json::from_str::<Value>(&inline).map_err(internal)?,
                        (_, Some(body)) => {
                            match serde_json::from_slice::<SessionPayloadBody>(&body)
                                .map_err(internal)?
                            {
                                SessionPayloadBody::Json(value) => value.0,
                                _ => return Err(AppError::Conflict(
                                    "reasoning lifecycle requires a canonical JSON payload".into(),
                                )),
                            }
                        }
                        _ => return Err(AppError::Conflict(
                            "reasoning lifecycle has no canonical payload".into(),
                        )),
                    };
                    let event: AgentEngineEvent = serde_json::from_value(
                        payload.get("event").cloned().ok_or_else(|| AppError::Conflict(
                            "reasoning lifecycle has no typed Runtime event".into(),
                        ))?,
                    )
                    .map_err(internal)?;
                    if let Some(Some(step)) = event.reasoning_display_transition() {
                        active_message_id = Some(
                            super::engine_journal::canonical_thinking_step_message_id(&root, step)?,
                        );
                    }
                }
            }
        } else {
            return Err(AppError::Conflict(
                "reasoning projection has no owning canonical Turn".into(),
            ));
        }
        for projection in projections.iter_mut().filter(|projection| {
            projection.presentation_intent == "thinking" && projection.projection["turn_id"] == root
        }) {
            let active = active_message_id
                .as_deref()
                .is_some_and(|identity| projection.projection["correlation_id"] == identity);
            projection.projection["state"] = json!(if active { "streaming" } else { "recorded" });
            projection.semantic_digest = digest_payload(&projection.projection).map_err(internal)?.0;
        }
    }
    tx.commit().await.map_err(internal)?;
    Ok(())
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::engine_journal::{EngineJournalWrite, test_fixture};
    use nomifun_agent_session::AgentSessionStore;

    #[tokio::test]
    async fn cold_history_tracks_phase_handoffs_reopening_and_the_exact_turn_terminal() {
        let (journal, pool) = test_fixture().await;
        let session = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let events = [
            json!({"event":"reasoning_delta","step":1,"text":"Inspect. "}),
            json!({"event":"output_text_delta","step":1,"text":"Reading the source."}),
            json!({"event":"reasoning_delta","step":1,"text":"Verify. "}),
            json!({"event":"model_step_started","step":2,"operation_id":"model:2"}),
            json!({"event":"reasoning_delta","step":2,"text":"Check the result."}),
        ];
        for (index, event) in events.into_iter().enumerate() {
            journal.append(event.to_string(), None, EngineJournalWrite::Progress).await.unwrap();
            let (mut history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
            hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
            let active = history.iter().filter(|item| item.presentation_intent == "thinking"
                && item.projection["state"] == "streaming").collect::<Vec<_>>();
            if matches!(index, 0 | 2 | 4) {
                assert_eq!(active.len(), 1);
                assert_eq!(active[0].projection["content"], match index {
                    0 => "Inspect. ", 2 => "Inspect. Verify. ", _ => "Check the result.",
                });
            } else {
                assert!(active.is_empty(), "recorded handoff closes only the reasoning phase");
            }
        }
        // A cancellation request is not a terminal. Owner settlement closes
        // the remaining row even when the Runtime emits no final text.
        let cancellation = tokio_util::sync::CancellationToken::new();
        journal.attach_runtime(cancellation.clone()).unwrap();
        cancellation.cancel();
        tokio::task::yield_now().await;
        let (mut history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
        hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
        assert!(history.iter().any(|item| item.projection["state"] == "streaming"));
        store.cancel_active_turn(&session, "test-cancel".into(), "session_api".into()).await.unwrap();
        let (mut history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
        hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
        assert!(history.iter().filter(|item| item.presentation_intent == "thinking")
            .all(|item| item.projection["state"] == "recorded"));
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(), None, EngineJournalWrite::Cleanup).await.unwrap();
        journal.append(json!({"event":"turn_cancelled","model_steps":2}).to_string(), None, EngineJournalWrite::Terminal).await.unwrap();
        let (mut history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
        hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
        assert!(history.iter().filter(|item| item.presentation_intent == "thinking")
            .all(|item| item.projection["state"] == "recorded"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events").fetch_one(&pool).await.unwrap();
        hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
        assert_eq!(count, sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_events").fetch_one(&pool).await.unwrap());
    }

    #[tokio::test]
    async fn stored_reasoning_payload_and_paused_turn_use_the_same_lifecycle() {
        let (journal, pool) = test_fixture().await;
        let session = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        journal.append(json!({"event":"reasoning_delta","step":1,"text":"思考正文".repeat(8192)}).to_string(),
            None, EngineJournalWrite::Progress).await.unwrap();
        let payloads: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'runtime/progress-recorded' AND payload_id IS NOT NULL"
        ).bind(session.as_ref()).fetch_one(&pool).await.unwrap();
        assert_eq!(payloads, 1, "exercise the external canonical JSON payload path");
        let (mut history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
        hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
        assert!(history.iter().any(|item| item.presentation_intent == "thinking" && item.projection["state"] == "streaming"));
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(), None, EngineJournalWrite::Cleanup).await.unwrap();
        journal.append(json!({"event":"turn_paused","model_steps":1,"reason":"user_requested"}).to_string(),
            None, EngineJournalWrite::Terminal).await.unwrap();
        let (mut history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
        hydrate_thinking_lifecycle(&pool, &session, &mut history).await.unwrap();
        assert!(history.iter().filter(|item| item.presentation_intent == "thinking").all(|item| item.projection["state"] == "recorded"));
        assert_eq!(store.head(&session).await.unwrap().active_turn_id.as_deref(), Some("turn"),
            "closing display reasoning does not release the native paused Turn");
    }

    #[tokio::test]
    async fn a_missing_canonical_turn_cannot_be_inferred_completed_from_reasoning_content() {
        let (journal, pool) = test_fixture().await;
        let session = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        journal.append(json!({"event":"reasoning_delta","step":1,"text":"Inspect."}).to_string(),
            None, EngineJournalWrite::Progress).await.unwrap();
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let (history, _, _) = store.message_history_before(&session, None, 50).await.unwrap();
        let mut orphan = history.into_iter().find(|item| item.presentation_intent == "thinking").unwrap();
        orphan.projection["turn_id"] = json!(uuid::Uuid::now_v7().to_string());
        let error = hydrate_thinking_lifecycle(&pool, &session, std::slice::from_mut(&mut orphan)).await.unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        assert_eq!(orphan.projection["state"], "recorded", "failed reads do not synthesize a lifecycle");
    }
}
