//! The inactive-link outbox retires exact Attempts, never an entire Session.
use super::*;

async fn committed_target_tx(tx:&mut Transaction<'_,Sqlite>,cleanup:&PendingConversationCleanup,event:&AgentExecutionEventRow)
    ->Result<ConversationCleanupTurnTarget,DbError> {
    let unconfirmed=|reason:&str|ConversationCleanupTurnTarget::Unconfirmed {reason:reason.into()};
    if event.execution_id!=cleanup.execution_id||event.step_id.as_deref()!=Some(cleanup.step_id.as_str())
        ||event.attempt_id.as_deref()!=Some(cleanup.attempt_id.as_str())||event.actor_type!="system"
        ||event.on_behalf_of_user_id!=cleanup.user_id||event.actor_id.is_some()
        ||event.actor_conversation_id.is_some()||event.actor_attempt_id.is_some()
        ||!matches!(event.event_type.as_str(),"attempt_changed"|"decision_requested") {
        return Ok(unconfirmed("cleanup source is not this original owner's System Attempt settlement"));
    }
    let payload:serde_json::Value=match serde_json::from_str(&event.payload) {
        Ok(serde_json::Value::Object(value))=>serde_json::Value::Object(value),
        _=>return Ok(unconfirmed("cleanup source payload is invalid")),
    };
    for field in ["turn_operation_key","canonical_operation_id","terminal_event_id"] {
        if payload.get(field).is_some_and(|value|value.as_str().is_none_or(|value|value.trim().is_empty()||value.chars().any(char::is_control))) {
            return Ok(unconfirmed("cleanup source has an invalid original invocation reference"));
        }
    }
    let prefix=format!("turn:user:{}:{}:",cleanup.user_id,cleanup.conversation_id);
    let key=payload.get("turn_operation_key").and_then(serde_json::Value::as_str);
    let canonical=payload.get("canonical_operation_id").and_then(serde_json::Value::as_str);
    let operation=match (key,canonical) {
        (Some(key),Some(operation)) if operation!=format!("{prefix}{key}")=>return Ok(unconfirmed("cleanup source invocation references disagree")),
        (_,Some(operation))=>operation.to_owned(),
        (Some(key),None)=>format!("{prefix}{key}"),
        (None,None)=>return Ok(unconfirmed("cleanup settlement lost its exact original invocation reference")),
    };
    if operation.strip_prefix(&prefix).is_none_or(|key|key.trim().is_empty()) {
        return Ok(unconfirmed("cleanup source points outside its original Session owner"));
    }
    if canonical.is_some() {
        let Some(terminal)=payload.get("terminal_event_id").and_then(serde_json::Value::as_str) else {
            return Ok(unconfirmed("canonical cleanup source lost its original terminal reference"));
        };
        // The formal DecisionRequested/Cancelled writer recorded this exact
        // canonical terminal. A damaged or foreign reference cannot select a
        // different live Turn merely because its spelling has a valid prefix.
        let source:Option<(String,String,i64)>=sqlx::query_as("SELECT turn.state,terminal.kind,terminal.kind_version \
            FROM agent_turns turn JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id \
            WHERE turn.session_id=? AND turn.operation_id=? AND turn.terminal_event_id=? AND terminal.correlation_id=turn.operation_id")
            .bind(&cleanup.conversation_id).bind(&operation).bind(terminal).fetch_optional(&mut **tx).await?;
        let valid=source.is_some_and(|(state,kind,version)|version==1&&match (state.as_str(),kind.as_str()) {
            ("completed","turn/completed")=>true,
            ("failed","turn/failed")|("cancelled","turn/cancelled")=>event.event_type!="decision_requested",
            _=>false,
        });
        if !valid {return Ok(unconfirmed("cleanup source no longer matches its original canonical terminal"));}
    }
    Ok(ConversationCleanupTurnTarget::Exact {operation_id:operation})
}

pub(super) async fn target_tx(tx:&mut Transaction<'_,Sqlite>,cleanup:&PendingConversationCleanup)
    ->Result<ConversationCleanupTurnTarget,DbError> {
    let raw:Option<Option<String>>=sqlx::query_scalar(
        "SELECT runtime_state FROM agent_execution_attempts WHERE execution_id=? AND step_id=? AND attempt_id=?",
    ).bind(&cleanup.execution_id).bind(&cleanup.step_id).bind(&cleanup.attempt_id).fetch_optional(&mut **tx).await?;
    let Some(raw)=raw else {return Ok(ConversationCleanupTurnTarget::Unconfirmed {reason:"cleanup lost its original Attempt facts".into()});};
    let effects=match AttemptConversationEffects::decode(raw.as_deref()) {
        Ok(effects)=>effects,Err(error)=>return Ok(ConversationCleanupTurnTarget::Unconfirmed {reason:error.to_string()}),
    };
    let key=match effects.current_turn_operation_key(&cleanup.attempt_id) {
        Ok(key)=>key,Err(error)=>return Ok(ConversationCleanupTurnTarget::Unconfirmed {reason:error.to_string()}),
    };
    let mut settled_target=None;
    if effects.pending_conversation_effects.is_empty() {
        // Settlement clears the live effect queue, not its original invocation
        // reference. Read only that Attempt's latest explicit source event.
        let saved:Option<AgentExecutionEventRow>=sqlx::query_as(
            "SELECT * FROM agent_execution_events \
             WHERE execution_id=? AND step_id=? AND attempt_id=? \
               AND event_type IN ('attempt_changed','decision_requested') \
               AND CASE WHEN json_valid(payload)=0 THEN 1 WHEN json_type(payload)<>'object' THEN 1 ELSE event_type='decision_requested' \
                 OR json_type(payload,'$.turn_operation_key') IS NOT NULL OR json_type(payload,'$.canonical_operation_id') IS NOT NULL END \
             ORDER BY sequence DESC LIMIT 1",
        ).bind(&cleanup.execution_id).bind(&cleanup.step_id).bind(&cleanup.attempt_id).fetch_optional(&mut **tx).await?;
        if let Some(saved)=saved {
            match committed_target_tx(tx,cleanup,&saved).await? {
                ConversationCleanupTurnTarget::Exact {operation_id}=>settled_target=Some(operation_id),
                unconfirmed@ConversationCleanupTurnTarget::Unconfirmed {..}=>return Ok(unconfirmed),
            }
        }
    }
    if key.trim().is_empty() {return Ok(ConversationCleanupTurnTarget::Unconfirmed {reason:"cleanup has an empty original invocation key".into()});}
    let continuing=effects.pending_conversation_effects.iter().any(|effect|matches!(effect,crate::models::PendingConversationEffect::DecisionInput {..}));
    let pending_target=(!continuing).then(||effects.pending_conversation_effects.iter().rev().find_map(|effect|match effect {
        crate::models::PendingConversationEffect::StopTurn {target_operation_id,..}
        |crate::models::PendingConversationEffect::Steer {target_operation_id,..}=>Some(target_operation_id.clone()),_=>None,
    })).flatten();
    let prefix=format!("turn:user:{}:{}:",cleanup.user_id,cleanup.conversation_id);
    let operation_id=pending_target.or(settled_target).unwrap_or_else(||format!("{prefix}{key}"));
    if operation_id.strip_prefix(&prefix).is_none_or(|key|key.trim().is_empty()) {
        return Ok(ConversationCleanupTurnTarget::Unconfirmed {reason:"cleanup target does not belong to its original Session owner".into()});
    }
    Ok(ConversationCleanupTurnTarget::Exact {operation_id})
}

pub(super) async fn list(pool:&SqlitePool,execution_id:Option<&str>,cursor:Option<ConversationCleanupCursor>,limit:i64)
    ->Result<PendingConversationCleanupPage,DbError> {
    let limit=limit.clamp(1,500);
    let mut tx=pool.begin().await?;
    let through_link_id=match cursor {Some(cursor)=>cursor.through_link_id,None=>sqlx::query_scalar::<_,i64>(
        "SELECT COALESCE(MAX(id),0) FROM conversation_execution_links").fetch_one(&mut *tx).await?};
    let rows:Vec<(i64,String,String,String,String,String)>=sqlx::query_as(
        "WITH pending_attempts AS (SELECT MIN(link.id) AS link_id FROM conversation_execution_links link \
         JOIN agent_executions execution ON execution.execution_id=link.execution_id \
         JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
         WHERE link.relation IN ('attempt','automation') AND link.active=0 AND link.cleanup_completed_at IS NULL \
           AND (? IS NULL OR link.execution_id=?) AND session.state<>'deleted' \
           AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=execution.user_id \
           AND NOT EXISTS(SELECT 1 FROM conversation_execution_links active_link \
             WHERE active_link.conversation_id=link.conversation_id AND active_link.execution_id=link.execution_id \
               AND active_link.step_id=link.step_id AND active_link.attempt_id=link.attempt_id \
               AND active_link.relation IN ('attempt','automation') AND active_link.active=1) \
         GROUP BY link.conversation_id,link.execution_id,link.step_id,link.attempt_id) \
         SELECT link.id,link.execution_id,execution.user_id,link.step_id,link.attempt_id,link.conversation_id \
         FROM pending_attempts pending JOIN conversation_execution_links link ON link.id=pending.link_id \
         JOIN agent_executions execution ON execution.execution_id=link.execution_id \
         WHERE link.id>? AND link.id<=? ORDER BY link.id LIMIT ?",
    ).bind(execution_id).bind(execution_id).bind(cursor.map_or(0,|cursor|cursor.after_link_id)).bind(through_link_id).bind(limit)
        .fetch_all(&mut *tx).await?;
    let mut cleanups=Vec::with_capacity(rows.len());
    for (link_id,execution_id,user_id,step_id,attempt_id,conversation_id) in rows {
        let mut cleanup=PendingConversationCleanup {link_id,execution_id,user_id,step_id,attempt_id,conversation_id,
            target:ConversationCleanupTurnTarget::Unconfirmed {reason:"original target not yet read".into()}};
        cleanup.target=target_tx(&mut tx,&cleanup).await?;
        cleanups.push(cleanup);
    }
    let next_cursor=(cleanups.len()==limit as usize).then(||ConversationCleanupCursor {
        after_link_id:cleanups.last().expect("full page").link_id,through_link_id});
    tx.commit().await?;
    Ok(PendingConversationCleanupPage {cleanups,next_cursor})
}

async fn validate_tx(tx:&mut Transaction<'_,Sqlite>,cleanup:&PendingConversationCleanup)->Result<bool,DbError> {
    // Acquire the original writer before reading either link or target. Only
    // reactivation of this same Attempt revokes its cleanup authority; another
    // Attempt may legitimately run in the same Session throughout exact Stop.
    let exact=sqlx::query(
        "UPDATE conversation_execution_links SET updated_at=updated_at \
         WHERE id=? AND execution_id=? AND step_id=? AND attempt_id=? AND conversation_id=? \
           AND relation IN ('attempt','automation') AND active=0 AND cleanup_completed_at IS NULL \
           AND EXISTS(SELECT 1 FROM agent_executions execution JOIN agent_sessions session \
             ON session.agent_session_id=conversation_execution_links.conversation_id \
             WHERE execution.execution_id=conversation_execution_links.execution_id AND execution.user_id=? \
               AND session.state<>'deleted' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
               AND json_extract(session.owner_ref_json,'$.principal_id')=?) \
           AND NOT EXISTS(SELECT 1 FROM conversation_execution_links active_link \
             WHERE active_link.conversation_id=conversation_execution_links.conversation_id \
               AND active_link.execution_id=conversation_execution_links.execution_id \
               AND active_link.step_id=conversation_execution_links.step_id \
               AND active_link.attempt_id=conversation_execution_links.attempt_id \
               AND active_link.relation IN ('attempt','automation') AND active_link.active=1)",
    ).bind(cleanup.link_id).bind(&cleanup.execution_id).bind(&cleanup.step_id).bind(&cleanup.attempt_id)
        .bind(&cleanup.conversation_id).bind(&cleanup.user_id).bind(&cleanup.user_id).execute(&mut **tx).await?;
    if exact.rows_affected()!=1 {return Ok(false);}
    let original=target_tx(tx,cleanup).await?;
    if let ConversationCleanupTurnTarget::Unconfirmed {reason}=&original {return Err(conflict(reason));}
    Ok(original==cleanup.target)
}

pub(super) async fn validate(pool:&SqlitePool,cleanup:&PendingConversationCleanup)->Result<bool,DbError> {
    let mut tx=pool.begin().await?;
    let valid=validate_tx(&mut tx,cleanup).await?;
    tx.commit().await?;Ok(valid)
}

pub(super) async fn acknowledge(pool:&SqlitePool,cleanup:&PendingConversationCleanup,completed_at:i64)->Result<bool,DbError> {
    let mut tx=pool.begin().await?;
    if !validate_tx(&mut tx,cleanup).await? {tx.commit().await?;return Ok(false);}
    let result=sqlx::query(
        "UPDATE conversation_execution_links SET cleanup_completed_at=?,updated_at=? \
         WHERE conversation_id=? AND execution_id=? AND step_id=? AND attempt_id=? \
           AND relation IN ('attempt','automation') AND active=0 AND cleanup_completed_at IS NULL",
    ).bind(completed_at).bind(completed_at).bind(&cleanup.conversation_id).bind(&cleanup.execution_id)
        .bind(&cleanup.step_id).bind(&cleanup.attempt_id).execute(&mut *tx).await?;
    tx.commit().await?;Ok(result.rows_affected()>=1)
}
