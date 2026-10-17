//! Owner-scoped policy for an ordinary Turn's user-input handoff.
use nomifun_agent_contracts::{AgentSessionId, OperationId, PrincipalRef};
use nomifun_common::AppError;
use nomifun_db::sqlx::SqlitePool;

/// Check the exact active Turn before a Runtime commits a normal input request.
/// Lead navigation links do not make an ordinary Turn an Execution Attempt.
/// Policy/identity refusals are conflicts; database failures remain internal.
pub async fn validate_user_input_handoff_policy(
    pool: &SqlitePool, owner: &PrincipalRef, session: &AgentSessionId, operation: &OperationId,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|error| AppError::Internal(error.to_string()))?;
    let admitted: bool = nomifun_db::sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_sessions session \
         JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
         JOIN agent_turns turn ON turn.session_id=session.agent_session_id AND turn.operation_id=? \
         WHERE session.agent_session_id=? AND session.state='live' \
           AND json_extract(session.owner_ref_json,'$.principal_kind')=? \
           AND json_extract(session.owner_ref_json,'$.principal_id')=? \
           AND head.status='running' AND head.active_turn_id=turn.operation_id \
           AND turn.state='running' AND turn.terminal_event_id IS NULL)",
    ).bind(operation.as_ref()).bind(session.as_ref()).bind(&owner.principal_kind).bind(&owner.principal_id)
        .fetch_one(&mut *tx).await.map_err(|error| AppError::Internal(error.to_string()))?;
    if !admitted { return Err(AppError::Conflict("User input handoff does not own the exact active Turn".into())); }
    let links: Vec<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = nomifun_db::sqlx::query_as(
        "SELECT execution.decision_policy,execution.status,attempt.status,step.status,attempt.attempt_id,attempt.runtime_state \
         FROM conversation_execution_links link \
         LEFT JOIN agent_executions execution ON execution.execution_id=link.execution_id \
           AND execution.user_id=? AND execution.deleted_at IS NULL \
         LEFT JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id \
           AND attempt.step_id=link.step_id AND attempt.attempt_id=link.attempt_id \
           AND attempt.attempt_no=(SELECT MAX(current.attempt_no) FROM agent_execution_attempts current \
               WHERE current.execution_id=link.execution_id AND current.step_id=link.step_id) \
         LEFT JOIN agent_execution_steps step ON step.execution_id=link.execution_id \
           AND step.step_id=link.step_id AND step.superseded_in_revision IS NULL \
         WHERE link.conversation_id=? AND link.active=1 AND link.relation IN ('attempt','automation') LIMIT 2",
    ).bind(&owner.principal_id).bind(session.as_ref()).fetch_all(&mut *tx)
        .await.map_err(|error| AppError::Internal(error.to_string()))?;
    match links.as_slice() {
        [] => Ok(()),
        [(Some(policy), Some(status), Some(attempt), Some(step), Some(attempt_id), runtime_state)]
            if matches!(policy.as_str(), "ask_user" | "automatic") && matches!(status.as_str(), "running" | "waiting_input")
                && attempt == "running" && step == "running" => {
            let effects = nomifun_db::AttemptConversationEffects::decode(runtime_state.as_deref())
                .map_err(|error| AppError::Internal(error.to_string()))?;
            let key = effects.current_turn_operation_key(attempt_id).map_err(|error| AppError::Internal(error.to_string()))?;
            if operation.as_ref() != format!("turn:user:{}:{}:{key}",owner.principal_id,session.as_ref()) {
                return Err(AppError::Conflict("User input handoff does not own the current Attempt invocation".into()));
            }
            Ok(())
        },
        _ => Err(AppError::Conflict("User input handoff has no unique active Attempt".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::sqlx::query;

    #[tokio::test]
    async fn policy_uses_real_schema_exact_owner_and_current_attempt_without_inventing_columns() {
        let owner = PrincipalRef { principal_kind:"user".into(),principal_id:nomifun_common::generate_id() };
        let db = nomifun_db::init_database_memory_with_owner(nomifun_common::UserId::parse(&owner.principal_id).unwrap()).await.unwrap();
        let session:AgentSessionId = nomifun_common::generate_id().into();
        let execution = nomifun_common::generate_id();
        let step = nomifun_common::generate_id();
        let attempt = nomifun_common::generate_id();
        let operation:OperationId = format!("turn:user:{}:{}:{attempt}:initial-turn",owner.principal_id,session.as_ref()).into();
        query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','policy',0,0,'{}',1,1)")
            .bind(session.as_ref()).bind(serde_json::to_string(&owner).unwrap()).execute(db.pool()).await.unwrap();
        query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running',?,0,1,0)")
            .bind(session.as_ref()).bind(operation.as_ref()).execute(db.pool()).await.unwrap();
        query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,accepted_at,started_at) VALUES(?,?,?,?,'running',1,1)")
            .bind(session.as_ref()).bind(operation.as_ref()).bind(operation.as_ref()).bind(operation.as_ref()).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await.is_ok());
        let foreign = PrincipalRef {principal_id:nomifun_common::generate_id(),..owner.clone()};
        assert!(matches!(validate_user_input_handoff_policy(db.pool(),&foreign,&session,&operation).await,Err(AppError::Conflict(_))));
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&OperationId::from("source-message-uuid")).await.is_err());

        query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,created_at,updated_at) VALUES(?,?,'policy','running','automatic','fixed','automatic','disabled','{}',1,1)")
            .bind(&execution).bind(&owner.principal_id).execute(db.pool()).await.unwrap();
        query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,introduced_in_revision,created_at,updated_at) VALUES(?,?,'policy','task','agent','running',0,1,1)")
            .bind(&step).bind(&execution).execute(db.pool()).await.unwrap();
        query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,created_at,updated_at) VALUES(?,?,?,0,'running','initial',1,1)")
            .bind(&attempt).bind(&execution).bind(&step).execute(db.pool()).await.unwrap();
        query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,created_at,updated_at) VALUES(?,?,'attempt',?,?,1,1)")
            .bind(session.as_ref()).bind(&execution).bind(&step).bind(&attempt).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await.is_ok(),
            "automatic decisions never authorize fabricating a required human answer");
        query("UPDATE agent_executions SET decision_policy='ask_user' WHERE execution_id=?").bind(&execution).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await.is_ok());
        let different:OperationId = format!("turn:user:{}:{}:different-input",owner.principal_id,session.as_ref()).into();
        query("UPDATE agent_turns SET operation_id=? WHERE session_id=?")
            .bind(different.as_ref()).bind(session.as_ref()).execute(db.pool()).await.unwrap();
        query("UPDATE agent_session_heads SET active_turn_id=? WHERE session_id=?")
            .bind(different.as_ref()).bind(session.as_ref()).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&different).await.is_err(),
            "the same Session's other active Turn cannot borrow this Attempt's handoff policy");
        query("UPDATE agent_turns SET operation_id=? WHERE session_id=?")
            .bind(operation.as_ref()).bind(session.as_ref()).execute(db.pool()).await.unwrap();
        query("UPDATE agent_session_heads SET active_turn_id=? WHERE session_id=?")
            .bind(operation.as_ref()).bind(session.as_ref()).execute(db.pool()).await.unwrap();
        query("UPDATE agent_execution_steps SET status='waiting_input' WHERE step_id=?").bind(&step).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await.is_err());
        query("UPDATE agent_execution_steps SET status='running' WHERE step_id=?").bind(&step).execute(db.pool()).await.unwrap();
        query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,created_at,updated_at) VALUES(?,?,?,1,'queued','retry',1,1)")
            .bind(nomifun_common::generate_id()).bind(&execution).bind(&step).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await.is_err());
        query("DELETE FROM conversation_execution_links WHERE conversation_id=?").bind(session.as_ref()).execute(db.pool()).await.unwrap();
        query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,created_at,updated_at) VALUES(?,?,'lead',1,1)")
            .bind(session.as_ref()).bind(&execution).execute(db.pool()).await.unwrap();
        assert!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await.is_ok());
        db.pool().close().await;
        assert!(matches!(validate_user_input_handoff_policy(db.pool(),&owner,&session,&operation).await,Err(AppError::Internal(_))));
    }
}
