//! Exact task retirement reads the original rows and events in one snapshot.
//! No ordinary visibility change, new ledger, cleanup acknowledgement or work.
use super::*;

pub(super) async fn read(pool:&SqlitePool,user_id:&str,conversation_id:&str,execution_id:&str,
    step_id:&str,attempt_id:&str)->Result<Option<AgentExecutionTaskRetirementFacts>,DbError> {
    let mut tx=pool.begin().await?;
    // A terminal/deleted aggregate may retain its original task evidence.
    // Only task owners in this same Session retain its Browser authority;
    // aggregate lead navigation does not grant another Session that purpose.
    let execution:Option<AgentExecutionRow>=sqlx::query_as("SELECT execution.* FROM agent_executions execution \
        JOIN agent_sessions session ON session.agent_session_id=? \
        WHERE execution.execution_id=? AND execution.user_id=? AND session.state='live' \
          AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
          AND json_extract(session.owner_ref_json,'$.principal_id')=? \
          AND NOT EXISTS(SELECT 1 FROM conversation_execution_links active_link WHERE active_link.conversation_id=session.agent_session_id \
            AND active_link.relation IN ('attempt','automation') AND active_link.active=1)")
        .bind(conversation_id).bind(execution_id).bind(user_id).bind(user_id).fetch_optional(&mut *tx).await?;
    let Some(execution)=execution else {tx.commit().await?;return Ok(None);};
    let link_id:Option<i64>=sqlx::query_scalar("SELECT MIN(id) FROM conversation_execution_links \
        WHERE execution_id=? AND step_id=? AND attempt_id=? AND conversation_id=? \
          AND relation IN ('attempt','automation') AND active=0")
        .bind(execution_id).bind(step_id).bind(attempt_id).bind(conversation_id).fetch_one(&mut *tx).await?;
    let Some(link_id)=link_id else {tx.commit().await?;return Ok(None);};
    let sessions:i64=sqlx::query_scalar("SELECT COUNT(DISTINCT conversation_id) FROM conversation_execution_links \
        WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation IN ('attempt','automation')")
        .bind(execution_id).bind(step_id).bind(attempt_id).fetch_one(&mut *tx).await?;
    if sessions!=1 {tx.commit().await?;return Ok(None);}
    let step:Option<AgentExecutionStepRow>=sqlx::query_as("SELECT * FROM agent_execution_steps WHERE execution_id=? AND step_id=?")
        .bind(execution_id).bind(step_id).fetch_optional(&mut *tx).await?;
    let Some(step)=step else {tx.commit().await?;return Ok(None);};
    let mut attempts=attempt_details_tx(&mut tx,execution_id,Some(step_id)).await?;
    let Some(original_attempt)=attempts.iter().find(|attempt|attempt.attempt.attempt_id==attempt_id).cloned() else {tx.commit().await?;return Ok(None);};
    if original_attempt.conversation_id.as_deref()!=Some(conversation_id) {
        tx.commit().await?;return Ok(None);
    }
    let Some(current_attempt)=attempts.pop() else {tx.commit().await?;return Ok(None);};
    let cleanup=PendingConversationCleanup {link_id,execution_id:execution_id.into(),user_id:user_id.into(),
        step_id:step_id.into(),attempt_id:attempt_id.into(),conversation_id:conversation_id.into(),
        target:ConversationCleanupTurnTarget::Unconfirmed {reason:"original retirement target not yet read".into()}};
    // Share exactly the cleanup owner's continuation/Stop target resolution.
    // An acknowledgement is deliberately not required: the original cleanup
    // callback asks Browser to retire before it can acknowledge this link.
    let target=conversation_cleanup::target_tx(&mut tx,&cleanup).await?;
    let events=sqlx::query_as::<_,AgentExecutionEventRow>("SELECT * FROM agent_execution_events event \
        WHERE event.execution_id=? AND event.sequence<=? AND ( \
          (event.step_id=? AND ((event.attempt_id=? AND (json_type(event.payload,'$.turn_operation_key')='text' OR json_type(event.payload,'$.canonical_operation_id')='text')) \
            OR (event.attempt_id=? AND event.event_type='attempt_changed' AND event.actor_type='system' AND event.on_behalf_of_user_id=? \
              AND (json_extract(event.payload,'$.attempt_status') IN ('completed','failed','cancelled') \
                OR json_type(event.payload,'$.turn_operation_key')='text' OR json_type(event.payload,'$.canonical_operation_id')='text')) \
            OR (event.attempt_id=? AND event.event_type='attempt_changed' AND event.actor_type IN ('user','agent','system') \
              AND event.on_behalf_of_user_id=? AND json_extract(event.payload,'$.change')='output_adopted'))) \
          OR (event.step_id=? AND event.attempt_id IS NULL AND event.event_type='step_changed' AND event.actor_type='system' \
            AND event.on_behalf_of_user_id=? AND json_extract(event.payload,'$.status')='skipped' AND json_extract(event.payload,'$.reason')='dependency_failed') \
          OR (event.step_id=? AND event.attempt_id IS NULL AND event.event_type='step_changed' \
            AND json_extract(event.payload,'$.change')='step_snapshot_replaced' AND json_extract(event.payload,'$.after.plan_revision')=?) \
          OR (event.step_id IS NULL AND event.attempt_id IS NULL AND event.event_type='plan_changed' \
            AND json_extract(event.payload,'$.change') IN ('replanned','adjusted') AND json_extract(event.payload,'$.plan_revision')=? \
            AND EXISTS(SELECT 1 FROM json_each(event.payload,'$.superseded_step_ids') selected WHERE selected.type='text' AND selected.value=?)) \
          OR (event.event_type='step_changed' AND json_extract(event.payload,'$.action')='cancel_step' \
            AND EXISTS(SELECT 1 FROM json_each(event.payload,'$.cancelled_step_ids') selected WHERE selected.type='text' AND selected.value=?)) \
          OR (event.step_id IS NULL AND event.attempt_id IS NULL AND event.event_type='status_changed' \
            AND (json_extract(event.payload,'$.status') IN ('cancelled','failed') OR json_extract(event.payload,'$.reason')='original_automation_source_stopped')) \
          OR (event.step_id IS NULL AND event.attempt_id IS NULL AND event.event_type='deleted' AND json_extract(event.payload,'$.deleted')=1)) \
        ORDER BY event.sequence")
        .bind(execution_id).bind(execution.event_sequence).bind(step_id).bind(attempt_id)
        .bind(&current_attempt.attempt.attempt_id).bind(user_id)
        .bind(&current_attempt.attempt.attempt_id).bind(user_id)
        .bind(step_id).bind(user_id).bind(step_id).bind(step.superseded_in_revision)
        .bind(step.superseded_in_revision).bind(step_id).bind(step_id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(AgentExecutionTaskRetirementFacts {execution,original_attempt,detail:AgentExecutionStepDetailRow {step,current_attempt:Some(current_attempt)},target,events}))
}
