//! Read-only collaboration observations over the original domain rows. No
//! participant snapshots, launch configuration, or result bodies are hydrated.
use crate::{DbError, models::AgentExecutionEventRow};
use sqlx::{Sqlite, SqlitePool};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentExecutionObservationHeader {
    pub execution_id: String,
    pub initial_plan_input: String,
    pub status: String,
    pub event_sequence: i64,
    pub version: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentExecutionObservationStep {
    pub step_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub version: i64,
    pub superseded_in_revision: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentExecutionObservationAttempt {
    pub attempt_id: String,
    pub step_id: String,
    pub attempt_no: i64,
    pub status: String,
    pub question: Option<String>,
    pub error: Option<String>,
    pub output_summary: Option<String>,
    pub runtime_state: Option<String>,
    pub conversation_id: Option<String>,
    pub last_turn_operation_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionObservationState {
    pub steps: Vec<AgentExecutionObservationStep>,
    pub attempts: Vec<AgentExecutionObservationAttempt>,
    pub events: Vec<AgentExecutionEventRow>,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionObservationRows {
    pub execution: AgentExecutionObservationHeader,
    /// Absent only when the caller already observed this exact domain cursor.
    pub state: Option<AgentExecutionObservationState>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AgentExecutionObservationRevision {
    pub execution_id: String,
    pub event_sequence: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AgentSessionObservationRevision {
    pub session_id: String,
    pub state: String,
    pub last_seq: Option<i64>,
    pub status: Option<String>,
    pub active_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCollaborationObservationRevisions {
    pub executions: Vec<AgentExecutionObservationRevision>,
    pub sessions: Vec<AgentSessionObservationRevision>,
}

#[tracing::instrument(skip_all, fields(execution_id=execution))]
pub(super) async fn observation(
    pool: &SqlitePool, owner: &str, execution: &str, after: Option<i64>,
) -> Result<Option<AgentExecutionObservationRows>, DbError> {
    let observed=crate::DatabaseOperationGuard::new(pool);
    let mut tx = pool.begin().await?;
    observed.acquired();
    let header = sqlx::query_as::<_, AgentExecutionObservationHeader>(
        "SELECT execution_id,initial_plan_input,status,event_sequence,version,deleted_at \
         FROM agent_executions WHERE execution_id=? AND user_id=?",
    ).bind(execution).bind(owner).fetch_optional(&mut *tx).await?;
    let Some(header) = header else { tx.commit().await?; observed.released(); return Ok(None); };
    if after.is_some_and(|after| after < 0 || after > header.event_sequence) {
        return Err(DbError::Conflict("collaboration cursor is outside its original Execution".into()));
    }
    let state = if after == Some(header.event_sequence) { None } else {
        let steps = sqlx::query_as::<_, AgentExecutionObservationStep>(
            "SELECT step_id,title,kind,status,version,superseded_in_revision FROM agent_execution_steps \
             WHERE execution_id=? ORDER BY created_at,id",
        ).bind(execution).fetch_all(&mut *tx).await?;
        let attempts = sqlx::query_as::<_, AgentExecutionObservationAttempt>(
            "SELECT attempt.attempt_id,attempt.step_id,attempt.attempt_no,attempt.status,attempt.question, \
                attempt.error,attempt.output_summary,attempt.runtime_state, \
                (SELECT link.conversation_id FROM conversation_execution_links link \
                 WHERE link.execution_id=attempt.execution_id AND link.step_id=attempt.step_id \
                   AND link.attempt_id=attempt.attempt_id AND link.relation IN('attempt','automation') \
                 ORDER BY link.active DESC,link.updated_at DESC,link.id DESC LIMIT 1) AS conversation_id, \
                (SELECT json_extract(event.payload,'$.turn_operation_key') FROM agent_execution_events event \
                 WHERE event.execution_id=attempt.execution_id AND event.step_id=attempt.step_id \
                   AND event.attempt_id=attempt.attempt_id AND json_type(event.payload,'$.turn_operation_key')='text' \
                 ORDER BY event.sequence DESC LIMIT 1) AS last_turn_operation_key \
             FROM agent_execution_attempts attempt WHERE attempt.execution_id=? ORDER BY attempt.step_id,attempt.attempt_no",
        ).bind(execution).fetch_all(&mut *tx).await?;
        let events = sqlx::query_as::<_, AgentExecutionEventRow>(
            "SELECT * FROM agent_execution_events WHERE execution_id=? AND sequence>? AND sequence<=? ORDER BY sequence",
        ).bind(execution).bind(after.unwrap_or(0)).bind(header.event_sequence).fetch_all(&mut *tx).await?;
        let mut sequence=after.unwrap_or(0);
        for event in &events {
            sequence=sequence.checked_add(1).ok_or_else(||DbError::Init("collaboration event cursor overflow".into()))?;
            if event.sequence!=sequence {return Err(DbError::Init("collaboration observation has a missing domain event".into()));}
        }
        if sequence!=header.event_sequence {return Err(DbError::Init("collaboration observation has no committed cursor event".into()));}
        Some(AgentExecutionObservationState { steps, attempts, events })
    };
    tx.commit().await?;
    observed.released();
    Ok(Some(AgentExecutionObservationRows { execution: header, state }))
}

/// Both source and original child cursors participate: native settlement may
/// advance while the domain owner has not appended another Execution event.
#[tracing::instrument(skip_all, fields(execution_count=executions.len(),session_count=sessions.len()))]
pub(super) async fn revisions(
    pool: &SqlitePool, owner: &str, executions: &[String], sessions: &[String],
) -> Result<AgentCollaborationObservationRevisions, DbError> {
    let executions = serde_json::to_string(executions).map_err(|error| DbError::Init(error.to_string()))?;
    let sessions = serde_json::to_string(sessions).map_err(|error| DbError::Init(error.to_string()))?;
    let observed=crate::DatabaseOperationGuard::new(pool);
    let mut tx = pool.begin().await?;
    observed.acquired();
    let executions = sqlx::query_as::<Sqlite, AgentExecutionObservationRevision>(
        "SELECT execution_id,event_sequence,deleted_at FROM agent_executions \
         WHERE user_id=? AND execution_id IN(SELECT value FROM json_each(?)) ORDER BY execution_id",
    ).bind(owner).bind(executions).fetch_all(&mut *tx).await?;
    let sessions = sqlx::query_as::<Sqlite, AgentSessionObservationRevision>(
        "SELECT session.agent_session_id AS session_id,session.state,head.last_seq,head.status,head.active_turn_id \
         FROM agent_sessions session LEFT JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
         WHERE session.agent_session_id IN(SELECT value FROM json_each(?)) \
           AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=? ORDER BY session.agent_session_id",
    ).bind(sessions).bind(owner).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    observed.released();
    Ok(AgentCollaborationObservationRevisions { executions, sessions })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture()->(crate::Database,String,String) {
        let database=crate::init_database_memory().await.unwrap();
        let owner=uuid::Uuid::now_v7().to_string();let execution=uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,event_sequence,created_at,updated_at) VALUES(?,?,'observe','running','fixed','automatic','automatic','{}',1,1,1)")
            .bind(&execution).bind(&owner).execute(database.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'created','system',?,'{}',1)")
            .bind(&execution).bind(&owner).execute(database.pool()).await.unwrap();
        (database,owner,execution)
    }

    #[tokio::test]
    async fn unchanged_cursor_reads_no_launch_tables_and_changes_return_only_new_events() {
        let (database,owner,execution)=fixture().await;
        let first=observation(database.pool(),&owner,&execution,None).await.unwrap().unwrap();
        assert_eq!(first.state.unwrap().events.len(),1);
        // If an unchanged observation touches an unnecessary launch/configuration
        // table this schema trap fails, rather than relying on elapsed timings.
        sqlx::query("DROP TABLE agent_execution_participants").execute(database.pool()).await.unwrap();
        assert!(observation(database.pool(),&owner,&execution,Some(1)).await.unwrap().unwrap().state.is_none());
        sqlx::query("UPDATE agent_executions SET event_sequence=2,status='cancelled',deleted_at=2 WHERE execution_id=?")
            .bind(&execution).execute(database.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,2,'deleted','user',?,'{\"deleted\":true}',2)")
            .bind(&execution).bind(&owner).execute(database.pool()).await.unwrap();
        let next=observation(database.pool(),&owner,&execution,Some(1)).await.unwrap().unwrap();
        assert_eq!(next.execution.deleted_at,Some(2));
        let events=next.state.unwrap().events;
        assert_eq!(events.len(),1);assert_eq!(events[0].sequence,2);
        assert!(observation(database.pool(),"foreign-owner",&execution,None).await.unwrap().is_none());
        assert!(observation(database.pool(),&owner,&execution,Some(3)).await.is_err());
    }

    #[tokio::test]
    async fn native_and_source_cursors_change_without_a_domain_event_and_deleted_work_stays_visible() {
        let (database,owner,execution)=fixture().await;
        let source=uuid::Uuid::now_v7().to_string();let child=uuid::Uuid::now_v7().to_string();
        for session in [&source,&child] {
            sqlx::query("INSERT INTO agent_sessions(agent_session_id,state,owner_ref_json,agent_binding_json,title,archived,pinned,next_seq,created_at) VALUES(?,'live',?,'{}','observation',0,0,2,1)")
                .bind(session).bind(serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string()).execute(database.pool()).await.unwrap();
            sqlx::query("INSERT INTO agent_session_heads(session_id,last_seq,status,active_set_generation,unread_count) VALUES(?,1,'running',0,0)")
                .bind(session).execute(database.pool()).await.unwrap();
        }
        let ids=vec![execution.clone()];let sessions=vec![source.clone(),child.clone()];
        let before=revisions(database.pool(),&owner,&ids,&sessions).await.unwrap();
        sqlx::query("UPDATE agent_session_heads SET last_seq=2,status='ready' WHERE session_id=?")
            .bind(&child).execute(database.pool()).await.unwrap();
        let native=revisions(database.pool(),&owner,&ids,&sessions).await.unwrap();
        assert_eq!(before.executions,native.executions);assert_ne!(before.sessions,native.sessions);
        sqlx::query("UPDATE agent_session_heads SET last_seq=3 WHERE session_id=?").bind(&source).execute(database.pool()).await.unwrap();
        let steering=revisions(database.pool(),&owner,&ids,&sessions).await.unwrap();assert_ne!(native,steering);
        sqlx::query("UPDATE agent_executions SET deleted_at=2 WHERE execution_id=?").bind(&execution).execute(database.pool()).await.unwrap();
        let deleted=revisions(database.pool(),&owner,&ids,&sessions).await.unwrap();
        assert_ne!(steering.executions,deleted.executions);assert_eq!(deleted.executions.len(),1);
        let foreign=revisions(database.pool(),"foreign-owner",&ids,&sessions).await.unwrap();
        assert!(foreign.executions.is_empty()&&foreign.sessions.is_empty());
    }

    #[tokio::test]
    async fn a_missing_incremental_fact_cannot_be_accepted_as_a_new_snapshot() {
        let (database,owner,execution)=fixture().await;
        sqlx::query("UPDATE agent_executions SET event_sequence=3 WHERE execution_id=?")
            .bind(&execution).execute(database.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,3,'status_changed','system',?,'{\"status\":\"cancelled\"}',3)")
            .bind(&execution).bind(&owner).execute(database.pool()).await.unwrap();
        assert!(matches!(observation(database.pool(),&owner,&execution,Some(1)).await,Err(DbError::Init(_))));
    }
}
