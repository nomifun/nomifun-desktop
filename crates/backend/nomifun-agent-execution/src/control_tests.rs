//! Real-repository regressions for exact canonical control cancellation.

use super::*;
use nomifun_common::generate_id;
use nomifun_db::{Database, SqliteAgentExecutionRepository};
use serde_json::{Value, json};

struct Fixture {
    database: Database,
    repository: SqliteAgentExecutionRepository,
    owner: String,
    execution: String,
    expected_version: i64,
    control: AgentExecutionControlSource,
}

impl Fixture {
    fn actor(&self) -> AgentExecutionActor {
        AgentExecutionActor::User { user_id: self.owner.clone() }
    }

    fn command(&self) -> VersionedAgentExecutionCommand {
        VersionedAgentExecutionCommand { expected_version: self.expected_version }
    }

    async fn cancel(&self) -> Result<CanonicalControlCancellation, AppError> {
        cancel_from_source(&self.repository, &self.owner, &self.actor(), &self.execution,
            self.command(), self.control.clone()).await
    }

    async fn row(&self) -> nomifun_db::models::AgentExecutionRow {
        self.repository.get_execution(&self.owner, &self.execution).await.unwrap().unwrap()
    }

    async fn cancellation_events(&self) -> Vec<nomifun_db::models::AgentExecutionEventRow> {
        self.repository.list_events(&self.owner, &self.execution, 0, 500).await.unwrap()
            .into_iter().filter(|event| {
                let payload: Value = serde_json::from_str(&event.payload).unwrap();
                payload.get("canonical_control_source").is_some()
            }).collect()
    }
}

async fn fixture() -> Fixture {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
    let execution = generate_id();
    let session = generate_id();
    let initial_plan = json!({"mode":"explicit","source":{
        "agent_session_id":session,"turn_operation_id":"original-turn",
        "tool_operation_id":"original-tool","effect_id":"original-effect"
    },"plan":{"steps":[]}});
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','source',0,0,'{}',1,1)")
        .bind(&session).bind(json!({"principal_kind":"user","principal_id":owner}).to_string())
        .execute(database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,max_parallel,initial_plan_input,version,event_sequence,created_at,updated_at) VALUES(?,?,'captured background task','running','automatic','adaptive','ask_user','disabled',1,?,4,1,1,1)")
        .bind(&execution).bind(&owner).bind(initial_plan.to_string())
        .execute(database.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
        .bind(&session).bind(&execution).execute(database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'created','system',?,'{}',1)")
        .bind(&execution).bind(&owner).execute(database.pool()).await.unwrap();
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    Fixture { database, repository, owner, execution, expected_version: 4,
        control: AgentExecutionControlSource {agent_session_id:session,input_event_id:generate_id()} }
}

#[tokio::test]
async fn canonical_control_cancel_records_only_source_references_and_captured_version() {
    let fixture = fixture().await;
    let result = fixture.cancel().await.unwrap();
    assert!(result.committed);
    assert!(!result.receipt.replayed);
    assert_eq!(result.receipt.source, fixture.control);
    assert_eq!(result.receipt.execution_id, fixture.execution);
    assert_eq!(result.receipt.status, AgentExecutionStatus::Cancelled);
    let row = fixture.row().await;
    assert_eq!(row.status, "cancelled");
    assert_eq!(row.version, fixture.expected_version + 1);
    assert_eq!(result.receipt.execution_event_sequence, row.event_sequence);
    let events = fixture.cancellation_events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, AgentExecutionEventKind::StatusChanged.as_str());
    assert_eq!(events[0].actor_id.as_deref(), Some(fixture.owner.as_str()));
    let payload: Value = serde_json::from_str(&events[0].payload).unwrap();
    assert_eq!(payload, json!({
        "status":"cancelled", "reason":"cancelled_by_caller",
        "canonical_control_source":fixture.control,
        "canonical_control_expected_version":fixture.expected_version,
    }), "the Execution event references the canonical human input instead of copying its text");
    fixture.database.close().await;
}

#[tokio::test]
async fn original_control_replay_preserves_its_event_and_cannot_cancel_a_successor_epoch() {
    let fixture = fixture().await;
    let original = fixture.cancel().await.unwrap();
    let first_replay = fixture.cancel().await.unwrap();
    assert!(!first_replay.committed);
    assert!(first_replay.receipt.replayed);
    assert_eq!(first_replay.receipt.execution_event_sequence, original.receipt.execution_event_sequence);
    assert_eq!(fixture.cancellation_events().await.len(), 1);

    // Seed a later aggregate phase adversarially. The original source receipt
    // must remain absorbing even if a future owner has opened a new epoch.
    sqlx::query("UPDATE agent_executions SET status='running',version=version+1,event_sequence=event_sequence+1 WHERE execution_id=?")
        .bind(&fixture.execution).execute(fixture.database.pool()).await.unwrap();
    let successor = fixture.row().await;
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,?,'status_changed','system',?,'{\"status\":\"running\",\"reason\":\"successor_fixture\"}',2)")
        .bind(&fixture.execution).bind(successor.event_sequence).bind(&fixture.owner)
        .execute(fixture.database.pool()).await.unwrap();
    let replay = fixture.cancel().await.unwrap();
    assert!(!replay.committed);
    assert!(replay.receipt.replayed);
    assert_eq!(replay.receipt.execution_event_sequence, original.receipt.execution_event_sequence);
    let after = fixture.row().await;
    assert_eq!(after.status, "running");
    assert_eq!(after.version, successor.version);
    assert_eq!(after.event_sequence, successor.event_sequence);
    assert_eq!(fixture.cancellation_events().await.len(), 1);
    fixture.database.close().await;
}

#[tokio::test]
async fn fresh_control_with_a_stale_captured_version_fails_without_refresh_or_cancel() {
    let fixture = fixture().await;
    sqlx::query("UPDATE agent_executions SET version=version+1 WHERE execution_id=?")
        .bind(&fixture.execution).execute(fixture.database.pool()).await.unwrap();
    let before = fixture.row().await;
    assert!(fixture.cancel().await.is_err());
    let after = fixture.row().await;
    assert_eq!(after.status, "running");
    assert_eq!(after.version, before.version);
    assert_eq!(after.event_sequence, before.event_sequence);
    assert!(fixture.cancellation_events().await.is_empty());
    fixture.database.close().await;
}

#[tokio::test]
async fn concurrent_identical_control_commits_one_cancellation_and_returns_one_event() {
    let fixture = fixture().await;
    let (first, second) = tokio::join!(fixture.cancel(), fixture.cancel());
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(usize::from(first.committed) + usize::from(second.committed), 1);
    assert_eq!(usize::from(first.receipt.replayed) + usize::from(second.receipt.replayed), 1);
    assert_eq!(first.receipt.execution_event_sequence, second.receipt.execution_event_sequence);
    assert_eq!(first.receipt.source, second.receipt.source);
    assert_eq!(fixture.cancellation_events().await.len(), 1);
    let row = fixture.row().await;
    assert_eq!(row.version, fixture.expected_version + 1);
    assert_eq!(row.event_sequence, 2);
    fixture.database.close().await;
}

#[tokio::test]
async fn canonical_control_rejects_foreign_actor_owner_source_and_changed_replay_target() {
    let fixture = fixture().await;
    let foreign_owner = generate_id();
    let foreign_actor = AgentExecutionActor::User {user_id:foreign_owner.clone()};
    assert!(cancel_from_source(&fixture.repository, &fixture.owner, &foreign_actor,
        &fixture.execution, fixture.command(), fixture.control.clone()).await.is_err());
    assert!(cancel_from_source(&fixture.repository, &foreign_owner, &foreign_actor,
        &fixture.execution, fixture.command(), fixture.control.clone()).await.is_err());
    let mut foreign_source = fixture.control.clone();
    foreign_source.agent_session_id = generate_id();
    assert!(cancel_from_source(&fixture.repository, &fixture.owner, &fixture.actor(),
        &fixture.execution, fixture.command(), foreign_source).await.is_err());
    assert!(fixture.cancellation_events().await.is_empty());
    assert_eq!(fixture.row().await.status, "running");

    fixture.cancel().await.unwrap();
    let changed = VersionedAgentExecutionCommand {expected_version:fixture.expected_version + 1};
    assert!(cancel_from_source(&fixture.repository, &fixture.owner, &fixture.actor(),
        &fixture.execution, changed, fixture.control.clone()).await.is_err(),
        "the same human input cannot be rebound to a different captured aggregate version");
    assert_eq!(fixture.cancellation_events().await.len(), 1);
    fixture.database.close().await;
}

#[tokio::test]
async fn canonical_control_cancellation_retains_unknown_attempt_evidence_and_pending_cleanup() {
    let fixture = fixture().await;
    let step = generate_id();
    let attempt = generate_id();
    let child = generate_id();
    let review = json!({"pending_conversation_effects":[],"review_blocked":{
        "kind":"canonical_turn_outcome_unknown","operation_id":"unknown-child-turn",
        "receipt_state":"running","reason":"external tool result is unconfirmed"
    }}).to_string();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','unknown child',0,0,'{}',1,1)")
        .bind(&child).bind(json!({"principal_kind":"user","principal_id":fixture.owner}).to_string())
        .execute(fixture.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,agent_mode,status,introduced_in_revision,created_at,updated_at) VALUES(?,?,'unknown task','exact work','agent','normal','waiting_input',0,1,1)")
        .bind(&step).bind(&fixture.execution).execute(fixture.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,error,runtime_state,created_at,updated_at) VALUES(?,?,?,1,'waiting_input','initial','external result unknown',?,1,1)")
        .bind(&attempt).bind(&fixture.execution).bind(&step).bind(&review)
        .execute(fixture.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'attempt',?,?,0,1,1)")
        .bind(&child).bind(&fixture.execution).bind(&step).bind(&attempt)
        .execute(fixture.database.pool()).await.unwrap();
    fixture.cancel().await.unwrap();
    let retained: (String, String, Option<String>) = sqlx::query_as(
        "SELECT status,runtime_state,error FROM agent_execution_attempts WHERE attempt_id=?")
        .bind(&attempt).fetch_one(fixture.database.pool()).await.unwrap();
    assert_eq!(retained.0, "cancelled");
    assert_eq!(retained.1, review, "cancellation cannot resolve or erase the unknown original effect");
    assert_eq!(retained.2.as_deref(), Some("external result unknown"));
    let cleanup: (bool, Option<i64>) = sqlx::query_as(
        "SELECT active,cleanup_completed_at FROM conversation_execution_links WHERE attempt_id=?")
        .bind(&attempt).fetch_one(fixture.database.pool()).await.unwrap();
    assert!(!cleanup.0);
    assert!(cleanup.1.is_none(), "canonical cancellation does not prove native cleanup");
    assert_eq!(fixture.cancellation_events().await.len(), 1);
    fixture.database.close().await;
}

#[tokio::test]
async fn pure_control_receipt_lookup_never_turns_an_accepted_input_into_a_new_cancellation() {
    let fixture = fixture().await;
    let accepted = json!({"content":"cancel the captured work","files":[],"inject_skills":[],
        "background_control":{"kind":"cancel_execution","target":{
            "execution_id":fixture.execution,"expected_version":fixture.expected_version}}});
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'session_api',?,'message/user-accepted',1,?,?)")
        .bind(&fixture.control.agent_session_id).bind(&fixture.control.input_event_id)
        .bind(format!("control-input:{}",fixture.control.input_event_id))
        .bind(&fixture.control.input_event_id).bind(accepted.to_string())
        .execute(fixture.database.pool()).await.unwrap();
    sqlx::query("UPDATE agent_sessions SET next_seq=2 WHERE agent_session_id=?")
        .bind(&fixture.control.agent_session_id).execute(fixture.database.pool()).await.unwrap();
    let before = fixture.row().await;
    let events_before = fixture.repository.list_events(&fixture.owner, &fixture.execution, 0, 500).await.unwrap();
    let missing = lookup_receipt(&fixture.repository, &fixture.owner, &fixture.actor(),
        &fixture.execution, fixture.command(), &fixture.control).await.unwrap();
    assert!(missing.is_none(), "accepted control input alone is not a cancellation receipt");
    let after = fixture.row().await;
    assert_eq!((after.status,after.version,after.event_sequence),
        (before.status,before.version,before.event_sequence));
    let events_after = fixture.repository.list_events(&fixture.owner, &fixture.execution, 0, 500).await.unwrap();
    assert_eq!(events_after.iter().map(|event|(&event.event_type,&event.payload,event.sequence)).collect::<Vec<_>>(),
        events_before.iter().map(|event|(&event.event_type,&event.payload,event.sequence)).collect::<Vec<_>>());

    let cancellation = fixture.cancel().await.unwrap();
    sqlx::query("UPDATE agent_executions SET status='running',version=version+1,event_sequence=event_sequence+1 WHERE execution_id=?")
        .bind(&fixture.execution).execute(fixture.database.pool()).await.unwrap();
    let successor = fixture.row().await;
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,?,'status_changed','system',?,'{\"status\":\"running\",\"reason\":\"successor_fixture\"}',2)")
        .bind(&fixture.execution).bind(successor.event_sequence).bind(&fixture.owner)
        .execute(fixture.database.pool()).await.unwrap();
    let receipt = lookup_receipt(&fixture.repository, &fixture.owner, &fixture.actor(),
        &fixture.execution, fixture.command(), &fixture.control).await.unwrap().unwrap();
    assert!(receipt.replayed);
    assert_eq!(receipt.source,fixture.control);
    assert_eq!(receipt.execution_event_sequence,cancellation.receipt.execution_event_sequence);
    assert_eq!(receipt.status,AgentExecutionStatus::Cancelled);
    assert!(lookup_receipt(&fixture.repository,&fixture.owner,&AgentExecutionActor::system(),
        &fixture.execution,fixture.command(),&fixture.control).await.is_err());
    assert!(lookup_receipt(&fixture.repository,&fixture.owner,&fixture.actor(),&fixture.execution,
        VersionedAgentExecutionCommand {expected_version:fixture.expected_version+1},&fixture.control).await.is_err());
    let after_lookup = fixture.row().await;
    assert_eq!((after_lookup.status,after_lookup.version,after_lookup.event_sequence),
        (successor.status,successor.version,successor.event_sequence));
    assert_eq!(fixture.cancellation_events().await.len(),1);
    let accepted_after:String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?")
        .bind(&fixture.control.input_event_id).fetch_one(fixture.database.pool()).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&accepted_after).unwrap(),accepted);
    let session_event_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=?")
        .bind(&fixture.control.agent_session_id).fetch_one(fixture.database.pool()).await.unwrap();
    assert_eq!(session_event_count,1,"pure lookup cannot append execution or input facts");
    fixture.database.close().await;
}
