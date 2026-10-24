use nomifun_common::{
    AdaptationPolicy, AgentExecutionEventKind, AgentExecutionStatus, AgentToolPolicy,
    DecisionPolicy, DelegationPolicy, ExecutionStepKind, ExecutionStepStatus, StepFailurePolicy,
};
use nomifun_db::{
    AgentExecutionAttemptSessionKind, CreateAgentExecutionAttemptParams,
    AgentExecutionAttemptRecoveryDisposition, AgentExecutionLeaseToken,
    RecoveredAgentExecutionAttemptOutput,
    CreateAgentExecutionParams, IAgentExecutionRepository,
    NewAgentExecutionEvent, NewAgentExecutionParticipant, NewAgentExecutionStep,
    NewAgentExecutionStepDependency, ReconcileAgentExecutionPlanParams,
    SqliteAgentExecutionRepository,
};

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";
const SOURCE_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";
const CANONICAL_LEAD_ID: &str = "0190f5fe-7c00-7a00-8000-000000000115";
const FOREIGN_CANONICAL_LEAD_ID: &str = "0190f5fe-7c00-7a00-8000-000000000116";
const DELETED_CANONICAL_LEAD_ID: &str = "0190f5fe-7c00-7a00-8000-000000000117";

async fn database() -> nomifun_db::Database {
    let database = nomifun_db::init_database_memory_with_owner(
        nomifun_common::UserId::parse(OWNER_ID).unwrap(),
    )
    .await
    .unwrap();
    database_with_provider(database).await
}

async fn file_database(path: &std::path::Path) -> (nomifun_db::Database, String) {
    let database = nomifun_db::init_database(path).await.unwrap();
    let owner_id = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
    (database_with_provider(database).await, owner_id)
}

async fn database_with_provider(database: nomifun_db::Database) -> nomifun_db::Database {
    nomifun_db::sqlx::query(
        "INSERT INTO providers (\
            provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
            created_at, updated_at\
         ) VALUES (?, 'openai', 'Fixture provider', 'https://example.invalid', \
                   'bearer', '', 1, 1, 1)",
    )
    .bind(PROVIDER_ID)
    .execute(database.pool())
    .await
    .unwrap();
    database
}

#[tokio::test]
async fn cleanup_keyset_passes_a_failed_full_page_and_bounds_continuous_arrivals() {
    let titles=(0..102).map(|index|format!("cleanup-{index}")).collect::<Vec<_>>();
    let fixture=scoped_stop::Fixture::memory(&titles.iter().map(String::as_str).collect::<Vec<_>>(),&[]).await;
    let db=&fixture.db;let repository=&fixture.repository;
    let mut conversations=Vec::new();
    for title in &titles[..101] {
        let started=fixture.start(title).await;
        conversations.push(started.current_attempt.as_ref().unwrap().conversation_id.clone().unwrap());
        fixture.complete(title).await;
    }
    // Repeated links of one original Attempt are a single cleanup unit.
    nomifun_db::sqlx::query("INSERT INTO conversation_execution_links (conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) SELECT conversation_id,execution_id,relation,step_id,attempt_id,0,1,1 FROM conversation_execution_links WHERE conversation_id=? LIMIT 1")
        .bind(&conversations[0]).execute(db.pool()).await.unwrap();
    let first=repository.list_pending_conversation_cleanups(None,None,100).await.unwrap();
    assert_eq!(first.cleanups.len(),100);assert_eq!(first.cleanups[0].conversation_id,conversations[0]);
    let cursor=first.next_cursor.unwrap();
    for cleanup in &first.cleanups {assert!(repository.validate_conversation_cleanup(cleanup).await.unwrap());}
    let arrival=fixture.start(&titles[101]).await.current_attempt.unwrap().conversation_id.unwrap();
    fixture.complete(&titles[101]).await;
    let arrival_id:i64=nomifun_db::sqlx::query_scalar("SELECT id FROM conversation_execution_links WHERE conversation_id=?")
        .bind(&arrival).fetch_one(db.pool()).await.unwrap();
    assert!(arrival_id>cursor.through_link_id);
    nomifun_db::sqlx::query("UPDATE conversation_execution_links SET updated_at=999999 WHERE id<=?")
        .bind(cursor.after_link_id).execute(db.pool()).await.unwrap();
    let tail=repository.list_pending_conversation_cleanups(None,Some(cursor),100).await.unwrap();
    assert_eq!(tail.cleanups.len(),1,"duplicate links do not hide item 101");
    assert_eq!(tail.cleanups[0].conversation_id,conversations[100]);assert!(tail.next_cursor.is_none());
    assert!(repository.mark_conversation_cleanup_completed_exact(&tail.cleanups[0],10000).await.unwrap());
    let retry=repository.list_pending_conversation_cleanups(None,None,100).await.unwrap();
    assert_eq!(retry.cleanups.iter().map(|cleanup|cleanup.link_id).collect::<Vec<_>>(),first.cleanups.iter().map(|cleanup|cleanup.link_id).collect::<Vec<_>>());
    let next=repository.list_pending_conversation_cleanups(None,retry.next_cursor,100).await.unwrap();
    assert_eq!(next.cleanups.len(),1);assert_eq!(next.cleanups[0].conversation_id,arrival);
    assert!(repository.mark_conversation_cleanup_completed_exact(&next.cleanups[0],10001).await.unwrap());
    assert!(repository.mark_conversation_cleanup_completed_exact(&retry.cleanups[0],10002).await.unwrap());
    let remaining:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM conversation_execution_links WHERE conversation_id=? AND cleanup_completed_at IS NULL")
        .bind(&conversations[0]).fetch_one(db.pool()).await.unwrap();
    assert_eq!(remaining,0,"only duplicate links of that same Attempt are acknowledged together");
}
async fn claimed_autowork_requirement(db:&nomifun_db::Database,session:&str)->nomifun_db::models::RequirementRow {
    use nomifun_db::IRequirementRepository;
    let repository=nomifun_db::SqliteRequirementRepository::new(db.pool().clone());
    let tag=nomifun_common::generate_id();
    repository.insert(&nomifun_db::models::NewRequirementRow {title:"Original AutoWork claim".into(),content:"same original work".into(),tag:tag.clone(),
        order_key:String::new(),sort_seq:String::new(),status:"pending".into(),priority:0,completion_note:None,
        owner_conversation_id:None,owner_terminal_id:None,active_turn_started_at:None,lease_expires_at:None,started_at:None,completed_at:None,
        attempt_count:0,created_by:"user".into(),extra:"{}".into(),created_at:1,updated_at:1}).await.unwrap();
    repository.claim_next_for_runner(&tag,Some(session),None,30_000,2).await.unwrap().unwrap().row
}

#[tokio::test]
async fn autowork_attempt_reuses_lead_without_becoming_disposable_attempt_transcript() {
    let db = database().await;
    insert_canonical_session(&db, CANONICAL_LEAD_ID, OWNER_ID, "live").await;
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES (?,'ready',NULL,0,0,0)")
        .bind(CANONICAL_LEAD_ID).execute(db.pool()).await.unwrap();
    let claim=claimed_autowork_requirement(&db,CANONICAL_LEAD_ID).await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let participant_id = nomifun_common::generate_id();
    let mut params = execution_params();
    params.lead_conversation_id = Some(CANONICAL_LEAD_ID.to_owned());
    params.initial_plan_input = serde_json::json!({
        "mode": "automation",
        "source": {
            "requirement_id": claim.requirement_id,
            "claim_generation": claim.claim_generation,
            "operation_id": "autowork:test"
        },
        "plan": { "steps": [] }
    })
    .to_string();
    let created = repository
        .create_execution_with_participants(
            OWNER_ID,
            &params,
            &[participant(participant_id.clone())],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap();
    let step_id = nomifun_common::generate_id();
    let planned = repository
        .reconcile_plan(
            OWNER_ID,
            &created.execution_id,
            created.version,
            &ReconcileAgentExecutionPlanParams {
                goal: None,
                adaptation_policy: None,
                decision_policy: None,
                delegation_policy: None,
                keep_step_ids: Vec::new(),
                new_participants: Vec::new(),
                retire_participant_ids: Vec::new(),
                new_steps: vec![step(step_id, Some(participant_id), "autowork")],
                new_dependencies: Vec::new(),
                execution_status: AgentExecutionStatus::Running,
            },
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            OWNER_ID,
            &created.execution_id,
            &planned.steps[0].step_id,
            planned.steps[0].version,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some(planned.participants[0].participant_id.clone()),
                start_immediately: false,
                trigger_reason: "initial".to_owned(),
                effective_config: r#"{"session_kind":"automation"}"#.to_owned(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt = queued.current_attempt.as_ref().unwrap().attempt.clone();
    // Domain linking shares the database writer with owner input/resume. It
    // cannot turn a parked user Session into an Attempt while that input is
    // being prepared, nor leave an active link after failed Turn admission.
    for status in ["paused","reconciliation","running"] {
        nomifun_db::sqlx::query("UPDATE agent_session_heads SET status=?,active_turn_id='original-user-turn' WHERE session_id=?")
            .bind(status).bind(CANONICAL_LEAD_ID).execute(db.pool()).await.unwrap();
        assert!(repository.start_attempt(OWNER_ID,&created.execution_id,&planned.steps[0].step_id,queued.step.version,
            &attempt.attempt_id,attempt.version,CANONICAL_LEAD_ID,AgentExecutionAttemptSessionKind::AutomationLead,None, None,
            &event(AgentExecutionEventKind::AttemptChanged)).await.is_err(),"{status} cannot acquire automation input ownership");
        assert_eq!(repository.list_conversation_links(OWNER_ID,&created.execution_id).await.unwrap().len(),1);
    }
    nomifun_db::sqlx::query("UPDATE agent_session_heads SET status='ready',active_turn_id=NULL WHERE session_id=?")
        .bind(CANONICAL_LEAD_ID).execute(db.pool()).await.unwrap();
    let running = repository
        .start_attempt(
            OWNER_ID,
            &created.execution_id,
            &planned.steps[0].step_id,
            queued.step.version,
            &attempt.attempt_id,
            attempt.version,
            CANONICAL_LEAD_ID,
            AgentExecutionAttemptSessionKind::AutomationLead,
            None, None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();

    assert_eq!(
        running.current_attempt.unwrap().conversation_id.as_deref(),
        Some(CANONICAL_LEAD_ID)
    );
    let links = repository
        .list_conversation_links(OWNER_ID, &created.execution_id)
        .await
        .unwrap();
    assert_eq!(links.len(), 2);
    assert!(links.iter().any(|link| link.relation == "lead"));
    assert!(links.iter().any(|link| {
        link.relation == "automation"
            && link.conversation_id == CANONICAL_LEAD_ID
            && link.attempt_id.as_deref() == Some(attempt.attempt_id.as_str())
    }));
    assert!(
        !repository
            .has_attempt_conversation_link(OWNER_ID, CANONICAL_LEAD_ID)
            .await
            .unwrap(),
        "the user's main AgentSession must never become a disposable Attempt transcript"
    );
}

fn event(kind: AgentExecutionEventKind) -> NewAgentExecutionEvent {
    NewAgentExecutionEvent {
        event_type: kind,
        step_id: None,
        attempt_id: None,
        actor: nomifun_common::AgentExecutionActor::system(),
        payload: "{}".to_owned(),
    }
}

fn participant(participant_id: impl Into<String>) -> NewAgentExecutionParticipant {
    NewAgentExecutionParticipant {
        participant_id: participant_id.into(),
        source_agent_id: SOURCE_AGENT_ID.to_owned(),
        preset_id: None,
        preset_revision: None,
        agent_snapshot: None,
        provider_id: Some(PROVIDER_ID.to_owned()),
        model: Some("model_test".to_owned()),
        role: Some("builder".to_owned()),
        capability: Some(r#"{"coding":true}"#.to_owned()),
        constraints: Some(r#"{"max_concurrency":1}"#.to_owned()),
        description: Some("repository fixture".to_owned()),
        system_prompt: None,
        enabled_skills: "[]".to_owned(),
        disabled_builtin_skills: "[]".to_owned(),
        sort_order: 0,
    }
}

fn step(
    step_id: impl Into<String>,
    assigned_participant_id: Option<String>,
    title: &str,
) -> NewAgentExecutionStep {
    NewAgentExecutionStep {
        step_id: step_id.into(),
        title: title.to_owned(),
        spec: format!("execute {title}"),
        role: Some("builder".to_owned()),
        tool_policy: AgentToolPolicy::Full,
        kind: ExecutionStepKind::Agent,
        agent_mode: Some(nomifun_common::AgentStepMode::Normal),
        profile: Some("{}".to_owned()),
        fanout_group: None,
        control_policy: None,
        status: ExecutionStepStatus::Pending,
        assigned_participant_id,
        assignment_score: Some(1.0),
        assignment_rationale: Some("fixture".to_owned()),
        assignment_source: Some(nomifun_common::ParticipantAssignmentSource::Planner),
        assignment_locked: false,
        failure_policy: StepFailurePolicy::FailExecution,
        preset_prompt: None,
        graph_x: None,
        graph_y: None,
    }
}

fn execution_params() -> CreateAgentExecutionParams {
    CreateAgentExecutionParams {
        goal: "verify v3 row identity separation".to_owned(),
        status: AgentExecutionStatus::Planning,
        adaptation_policy: AdaptationPolicy::Fixed,
        decision_policy: DecisionPolicy::Automatic,
        delegation_policy: DelegationPolicy::Automatic,
        max_parallel: 2,
        work_dir: None,
        lead_conversation_id: None,
        initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
    }
}

async fn insert_canonical_session(
    database: &nomifun_db::Database,
    agent_session_id: &str,
    owner_id: &str,
    state: &str,
) {
    let owner_ref = serde_json::json!({
        "principal_kind": "user",
        "principal_id": owner_id,
    })
    .to_string();
    match state {
        "live" => {
            nomifun_db::sqlx::query(
                "INSERT INTO agent_sessions (\
                    agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, next_seq, created_at\
                 ) VALUES (?, ?, 'live', 'Canonical lead', 0, 0, '{}', 1, 1)",
            )
            .bind(agent_session_id)
            .bind(owner_ref)
            .execute(database.pool())
            .await
            .unwrap();
        }
        "deleted" => {
            nomifun_db::sqlx::query(
                "INSERT INTO agent_sessions (\
                    agent_session_id, owner_ref_json, state, deleted_at\
                 ) VALUES (?, ?, 'deleted', 1)",
            )
            .bind(agent_session_id)
            .bind(owner_ref)
            .execute(database.pool())
            .await
            .unwrap();
        }
        other => panic!("unsupported canonical Session fixture state {other}"),
    }
}

async fn create_execution(
    repository: &SqliteAgentExecutionRepository,
) -> nomifun_db::models::AgentExecutionRow {
    create_execution_for_owner(repository, OWNER_ID).await
}

async fn create_execution_for_owner(
    repository: &SqliteAgentExecutionRepository,
    owner_id: &str,
) -> nomifun_db::models::AgentExecutionRow {
    let participant_id = nomifun_common::generate_id();
    repository
        .create_execution_with_participants(
            owner_id,
            &execution_params(),
            &[participant(participant_id)],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn agent_execution_rows_expose_business_uuidv7_identity() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let created = create_execution(&repository).await;

    assert!(nomifun_common::AgentExecutionId::parse(&created.execution_id).is_ok());
    assert_eq!(created.user_id, OWNER_ID);
    assert_eq!(created.goal, "verify v3 row identity separation");
    assert_eq!(created.status, "planning");

    let fetched = repository
        .get_execution(OWNER_ID, &created.execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.execution_id, created.execution_id);
    assert_eq!(fetched.user_id, created.user_id);
    assert_eq!(fetched.goal, created.goal);
    assert_eq!(fetched.status, created.status);
    assert_eq!(fetched.version, created.version);
}

#[tokio::test]
async fn store_only_canonical_session_can_be_persisted_as_execution_lead() {
    let db = database().await;
    insert_canonical_session(&db, CANONICAL_LEAD_ID, OWNER_ID, "live").await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let mut params = execution_params();
    params.lead_conversation_id = Some(CANONICAL_LEAD_ID.to_owned());
    let created_event = NewAgentExecutionEvent {
        event_type: AgentExecutionEventKind::Created,
        step_id: None,
        attempt_id: None,
        actor: nomifun_common::AgentExecutionActor::agent(CANONICAL_LEAD_ID, None),
        payload: "{}".to_owned(),
    };

    let execution = repository
        .create_execution_with_participants(
            OWNER_ID,
            &params,
            &[participant(nomifun_common::generate_id())],
            &created_event,
        )
        .await
        .unwrap();

    let lead: (String, String, bool) = nomifun_db::sqlx::query_as(
        "SELECT conversation_id, relation, active \
         FROM conversation_execution_links WHERE execution_id = ?",
    )
    .bind(&execution.execution_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(lead, (CANONICAL_LEAD_ID.to_owned(), "lead".to_owned(), true));
    let actor: (String, Option<String>) = nomifun_db::sqlx::query_as(
        "SELECT actor_type, actor_conversation_id FROM agent_execution_events \
         WHERE execution_id = ? AND sequence = 1",
    )
    .bind(&execution.execution_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        actor,
        ("agent".to_owned(), Some(CANONICAL_LEAD_ID.to_owned()))
    );
    let persisted = repository
        .get_execution_detail(OWNER_ID, &execution.execution_id)
        .await
        .unwrap()
        .expect("canonical lead execution persists");
    assert_eq!(persisted.execution.execution_id, execution.execution_id);
}

#[tokio::test]
async fn canonical_execution_lead_rejects_foreign_and_deleted_sessions_atomically() {
    let db = database().await;
    insert_canonical_session(
        &db,
        FOREIGN_CANONICAL_LEAD_ID,
        "0190f5fe-7c00-7a00-8000-000000000099",
        "live",
    )
    .await;
    insert_canonical_session(&db, DELETED_CANONICAL_LEAD_ID, OWNER_ID, "deleted").await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());

    for lead in [FOREIGN_CANONICAL_LEAD_ID, DELETED_CANONICAL_LEAD_ID] {
        let mut params = execution_params();
        params.lead_conversation_id = Some(lead.to_owned());
        let error = repository
            .create_execution_with_participants(
                OWNER_ID,
                &params,
                &[participant(nomifun_common::generate_id())],
                &event(AgentExecutionEventKind::Created),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, nomifun_db::DbError::Conflict(ref message) if message.contains("lead AgentSession")),
            "unexpected canonical lead rejection: {error:?}"
        );
    }

    let persisted: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_executions",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(persisted, 0, "failed lead admission must roll back execution rows");
}

#[tokio::test]
async fn agent_execution_row_model_excludes_retired_plan_gate_mapping() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let created = create_execution(&repository).await;
    let columns: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('agent_executions')",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert!(!columns.iter().any(|column|column=="plan_gate"));
    let count: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_executions WHERE execution_id=?")
    .bind(&created.execution_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(count,1);
    let participant_id = repository
        .get_execution_detail(OWNER_ID, &created.execution_id)
        .await
        .unwrap()
        .expect("execution exists")
        .participants
        .first()
        .unwrap()
        .participant_id
        .clone();
    let step_id = nomifun_common::generate_id();

    let detail = repository
        .reconcile_plan(
            OWNER_ID,
            &created.execution_id,
            created.version,
            &ReconcileAgentExecutionPlanParams {
                goal: None,
                adaptation_policy: None,
                decision_policy: None,
                delegation_policy: None,
                keep_step_ids: Vec::new(),
                new_participants: Vec::new(),
                retire_participant_ids: Vec::new(),
                new_steps: vec![step(
                    step_id.clone(),
                    Some(participant_id.clone()),
                    "compile",
                )],
                new_dependencies: Vec::new(),
                execution_status: AgentExecutionStatus::Running,
            },
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();

    assert_eq!(detail.participants.len(), 1);
    assert_eq!(detail.steps.len(), 1);
    assert!(nomifun_common::validate_uuidv7(&detail.participants[0].participant_id).is_ok());
    assert!(nomifun_common::validate_uuidv7(&detail.steps[0].step_id).is_ok());
    assert_eq!(detail.participants[0].participant_id, participant_id);
    assert_eq!(detail.steps[0].step_id, step_id);
    assert_eq!(detail.participants[0].execution_id, created.execution_id);
    assert_eq!(detail.steps[0].execution_id, created.execution_id);
    assert_eq!(
        detail.steps[0].assigned_participant_id.as_deref(),
        Some(detail.participants[0].participant_id.as_str())
    );

    let conversation_id = nomifun_common::generate_id();
    insert_canonical_session(&db, &conversation_id, OWNER_ID, "live").await;

    let queued = repository
        .create_attempt(
            OWNER_ID,
            &created.execution_id,
            &detail.steps[0].step_id,
            detail.steps[0].version,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some(detail.participants[0].participant_id.clone()),
                start_immediately: false,
                trigger_reason: "initial".to_owned(),
                effective_config: "{}".to_owned(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let queued_attempt = queued.current_attempt.as_ref().unwrap().attempt.clone();
    assert!(nomifun_common::validate_uuidv7(&queued_attempt.attempt_id).is_ok());
    assert_eq!(queued_attempt.execution_id, created.execution_id);
    assert_eq!(queued_attempt.step_id, detail.steps[0].step_id);
    assert_eq!(
        queued_attempt.participant_id.as_deref(),
        Some(detail.participants[0].participant_id.as_str())
    );

    let running = repository
        .start_attempt(
            OWNER_ID,
            &created.execution_id,
            &detail.steps[0].step_id,
            queued.step.version,
            &queued_attempt.attempt_id,
            queued_attempt.version,
            &conversation_id,
            AgentExecutionAttemptSessionKind::ChildAttempt,
            None, None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let running_attempt = running.current_attempt.unwrap().attempt;
    assert_eq!(running_attempt.attempt_id, queued_attempt.attempt_id);

    let links = repository
        .list_conversation_links(OWNER_ID, &created.execution_id)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].execution_id, created.execution_id);
    assert_eq!(links[0].relation, "attempt");
    assert!(links[0].active);
    assert_eq!(
        links[0].step_id.as_deref(),
        Some(detail.steps[0].step_id.as_str())
    );
    assert_eq!(
        links[0].attempt_id.as_deref(),
        Some(running_attempt.attempt_id.as_str())
    );

    let events = repository
        .list_events(OWNER_ID, &created.execution_id, 0, 100)
        .await
        .unwrap();
    assert!(events.len() >= 3);
    assert!(events.iter().all(|row| row.sequence > 0));
    assert!(events.iter().all(|row| row.execution_id == created.execution_id));
    assert!(events
        .iter()
        .filter_map(|row| row.step_id.as_deref())
        .all(|id| nomifun_common::validate_uuidv7(id).is_ok()));
    assert!(events
        .iter()
        .filter_map(|row| row.attempt_id.as_deref())
        .all(|id| nomifun_common::validate_uuidv7(id).is_ok()));
    assert!(events
        .iter()
        .any(|row| row.step_id.as_deref() == Some(detail.steps[0].step_id.as_str())));
    assert!(events
        .iter()
        .any(|row| row.attempt_id.as_deref() == Some(running_attempt.attempt_id.as_str())));
}

#[tokio::test]
async fn agent_execution_business_ids_are_uuidv7_and_dependencies_use_them() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let created = create_execution(&repository).await;
    let participant_id = repository
        .get_execution_detail(OWNER_ID, &created.execution_id)
        .await
        .unwrap()
        .expect("execution exists")
        .participants[0]
        .participant_id
        .clone();
    let first_step_id = nomifun_common::generate_id();
    let second_step_id = nomifun_common::generate_id();

    let detail = repository
        .reconcile_plan(
            OWNER_ID,
            &created.execution_id,
            created.version,
            &ReconcileAgentExecutionPlanParams {
                goal: None,
                adaptation_policy: None,
                decision_policy: None,
                delegation_policy: None,
                keep_step_ids: Vec::new(),
                new_participants: Vec::new(),
                retire_participant_ids: Vec::new(),
                new_steps: vec![
                    step(
                        first_step_id.clone(),
                        Some(participant_id.clone()),
                        "first",
                    ),
                    step(
                        second_step_id.clone(),
                        Some(participant_id),
                        "second",
                    ),
                ],
                new_dependencies: vec![NewAgentExecutionStepDependency {
                    blocker_step_id: first_step_id.clone(),
                    blocked_step_id: second_step_id.clone(),
                }],
                execution_status: AgentExecutionStatus::Running,
            },
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();

    assert_eq!(detail.steps.len(), 2);
    assert_eq!(detail.dependencies.len(), 1);
    assert!(nomifun_common::validate_uuidv7(&detail.dependencies[0].blocker_step_id).is_ok());
    assert!(nomifun_common::validate_uuidv7(&detail.dependencies[0].blocked_step_id).is_ok());
    assert_eq!(detail.dependencies[0].execution_id, created.execution_id);
    assert_eq!(
        detail.dependencies[0].blocker_step_id,
        detail.steps
            .iter()
            .find(|row| row.title == "first")
            .unwrap()
            .step_id
    );
    assert_eq!(
        detail.dependencies[0].blocked_step_id,
        detail.steps
            .iter()
            .find(|row| row.title == "second")
            .unwrap()
            .step_id
    );

    let raw_columns: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('agent_execution_steps') ORDER BY cid",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(
        raw_columns,
        vec![
            "id",
            "step_id",
            "execution_id",
            "title",
            "spec",
            "role",
            "tool_policy",
            "kind",
            "agent_mode",
            "profile",
            "fanout_group",
            "control_policy",
            "delegation_depth",
            "status",
            "assigned_participant_id",
            "assignment_score",
            "assignment_rationale",
            "assignment_source",
            "assignment_locked",
            "failure_policy",
            "preset_prompt",
            "graph_x",
            "graph_y",
            "dispatch_after",
            "version",
            "introduced_in_revision",
            "superseded_in_revision",
            "created_at",
            "updated_at",
        ]
    );
}

async fn recovery_fixture(
    db: &nomifun_db::Database,
    repository: &SqliteAgentExecutionRepository,
) -> (String, nomifun_db::AgentExecutionStepDetailRow, RecoveredAgentExecutionAttemptOutput) {
    recovery_fixture_for_owner(db, repository, OWNER_ID).await
}

async fn recovery_fixture_for_owner(
    db: &nomifun_db::Database,
    repository: &SqliteAgentExecutionRepository,
    owner_id: &str,
) -> (String, nomifun_db::AgentExecutionStepDetailRow, RecoveredAgentExecutionAttemptOutput) {
    let created = create_execution_for_owner(repository, owner_id).await;
    let participant_id = repository.get_execution_detail(owner_id, &created.execution_id)
        .await.unwrap().unwrap().participants[0].participant_id.clone();
    let planned = repository.reconcile_plan(
        owner_id, &created.execution_id, created.version,
        &ReconcileAgentExecutionPlanParams {
            goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None,
            keep_step_ids: Vec::new(), new_participants: Vec::new(), retire_participant_ids: Vec::new(),
            new_steps: vec![step(nomifun_common::generate_id(), Some(participant_id.clone()), "recovery")],
            new_dependencies: Vec::new(), execution_status: AgentExecutionStatus::Running,
        }, &event(AgentExecutionEventKind::PlanChanged),
    ).await.unwrap();
    let queued = repository.create_attempt(
        owner_id, &created.execution_id, &planned.steps[0].step_id, planned.steps[0].version,
        None, &CreateAgentExecutionAttemptParams {
            participant_id: Some(participant_id), start_immediately: false,
            trigger_reason: "initial".to_owned(), effective_config: "{}".to_owned(),
            retry_after: None, runtime_state: None,
        }, &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    let attempt = queued.current_attempt.as_ref().unwrap().attempt.clone();
    let conversation_id = nomifun_common::generate_id();
    insert_canonical_session(db, &conversation_id, owner_id, "live").await;
    let running = repository.start_attempt(
        owner_id, &created.execution_id, &queued.step.step_id, queued.step.version,
        &attempt.attempt_id, attempt.version, &conversation_id,
        AgentExecutionAttemptSessionKind::ChildAttempt, None, None, &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    let operation_id = format!("turn:user:{owner_id}:{conversation_id}:{}:initial-turn", attempt.attempt_id);
    let terminal_event_id = nomifun_common::generate_id();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_events (session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) \
         VALUES (?,1,?,'recovery-fixture',?,'turn/completed',1,?,?)"
    ).bind(&conversation_id).bind(&terminal_event_id).bind(&terminal_event_id).bind(&operation_id)
        .bind(r#"{"model_steps":3,"finish_reason":"stop","finished_at_ms":10}"#)
        .execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_turns (session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,started_at,finished_at) \
         VALUES (?,?,?,?,'completed',?,1,1,1)"
    ).bind(&conversation_id).bind(&operation_id).bind(&operation_id).bind(&operation_id)
        .bind(&terminal_event_id).execute(db.pool()).await.unwrap();
    (created.execution_id, running, RecoveredAgentExecutionAttemptOutput {
        attempt_id: attempt.attempt_id, conversation_id, canonical_operation_id: operation_id,
        terminal_event_id, user_input_request: None, disposition:nomifun_db::AgentExecutionAttemptDisposition::Delivered, text: Some("已完成游戏实现".to_owned()),
        output_files: vec!["/workspace/index.html".to_owned()], error: None, tokens: Some(12),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_waiting_attempt_waits_for_writer_and_rechecks_versions_and_target() {
    use std::time::Duration;

    for mutation in [
        "unchanged",
        "execution_version",
        "step_version",
        "attempt_version",
        "attempt_cancelled",
        "session_owner",
        "inactive_link",
    ] {
        // Use the production file-backed WAL pool and its own canonical owner
        // from the start; no backup or snapshot mechanism is part of this test.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resume-contention.db");
        let (db, owner_id) = file_database(&path).await;
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution_id, running, _) = recovery_fixture_for_owner(&db, &repository, &owner_id).await;
        let attempt = &running.current_attempt.as_ref().unwrap().attempt;
        let waiting = repository
            .settle_attempt(
                &owner_id,
                &execution_id,
                &running.step.step_id,
                running.step.version,
                &attempt.attempt_id,
                attempt.version,
                None,
                &nomifun_db::SettleAgentExecutionAttemptParams {
                    expected_terminal_turn: None,
                    expected_active_session_turn: None,
                    attempt_status: nomifun_common::ExecutionAttemptStatus::WaitingInput,
                    step_status: ExecutionStepStatus::WaitingInput,
                    execution_status: Some(AgentExecutionStatus::WaitingInput),
                    question: Some(Some("continue?".to_owned())),
                    error: None,
                    output_summary: None,
                    output_files: None,
                    tokens: None,
                    retry_after: None,
                    runtime_state: Some(None),
                    started_at: None,
                    finished_at: None,
                    loop_repeat_reset: None,
                },
                &event(AgentExecutionEventKind::DecisionRequested),
            )
            .await
            .unwrap();

        let execution = repository
            .get_execution(&owner_id, &execution_id)
            .await
            .unwrap()
            .unwrap();
        let attempt = waiting.current_attempt.as_ref().unwrap().attempt.clone();
        let mut effects = nomifun_db::AttemptConversationEffects::default();
        effects.push_decision("continue-operation".into(), "continue".into()).unwrap();
        let runtime_state = effects.encode().unwrap();

        let mut writer = db.pool().begin().await.unwrap();
        nomifun_db::sqlx::query(
            "UPDATE agent_executions SET updated_at = updated_at WHERE execution_id = ?",
        )
        .bind(&execution_id)
        .execute(&mut *writer)
        .await
        .unwrap();

        let concurrent_execution_id = execution_id.clone();
        let concurrent_step_id = waiting.step.step_id.clone();
        let concurrent_attempt_id = attempt.attempt_id.clone();
        let concurrent_runtime_state = runtime_state.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let mut resume = tokio::spawn(async move {
            started.send(()).unwrap();
            repository.resume_waiting_attempt(
                &owner_id,
                &concurrent_execution_id,
                execution.version,
                &concurrent_step_id,
                waiting.step.version,
                &concurrent_attempt_id,
                attempt.version,
                &nomifun_db::AttemptConversationEffectParams {
                    runtime_state: Some(concurrent_runtime_state),
                },
                &event(AgentExecutionEventKind::DecisionAnswered),
            ).await
        });
        ready.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut resume).await.is_err(),
            "{mutation}: resume must wait for the writer instead of failing a read-to-write upgrade"
        );
        assert!(db.pool().size() >= 2, "resume must contend on a separate SQLite connection");

        let mutation_sql = match mutation {
            "unchanged" => None,
            "execution_version" => Some("UPDATE agent_executions SET version = version + 1 WHERE execution_id = ?"),
            "step_version" => Some("UPDATE agent_execution_steps SET version = version + 1 WHERE execution_id = ?"),
            "attempt_version" => Some("UPDATE agent_execution_attempts SET version = version + 1 WHERE execution_id = ?"),
            "attempt_cancelled" => Some("UPDATE agent_execution_attempts SET status = 'cancelled' WHERE execution_id = ?"),
            "session_owner" => Some("UPDATE agent_sessions SET owner_ref_json = json_set(owner_ref_json, '$.principal_id', '0190f5fe-7c00-7a00-8000-000000000116') WHERE agent_session_id IN (SELECT conversation_id FROM conversation_execution_links WHERE execution_id = ?)"),
            "inactive_link" => Some("UPDATE conversation_execution_links SET active = 0 WHERE execution_id = ? AND relation = 'attempt'"),
            _ => unreachable!(),
        };
        if let Some(sql) = mutation_sql {
            nomifun_db::sqlx::query(sql).bind(&execution_id).execute(&mut *writer).await.unwrap();
        }
        let state_sql = "SELECT execution.version, execution.status, step.version, step.status, \
            attempt.version, attempt.status, attempt.question, attempt.runtime_state, execution.event_sequence \
            FROM agent_executions execution \
            JOIN agent_execution_steps step ON step.execution_id = execution.execution_id \
            JOIN agent_execution_attempts attempt ON attempt.execution_id = execution.execution_id \
                AND attempt.step_id = step.step_id \
            WHERE execution.execution_id = ?";
        type StoredState = (i64, String, i64, String, i64, String, Option<String>, Option<String>, i64);
        let before: StoredState = nomifun_db::sqlx::query_as(state_sql)
            .bind(&execution_id).fetch_one(&mut *writer).await.unwrap();
        writer.commit().await.unwrap();

        let result = tokio::time::timeout(Duration::from_secs(6), resume)
            .await.expect("resume should finish within SQLite busy_timeout")
            .expect("resume task should not panic");
        let after: StoredState = nomifun_db::sqlx::query_as(state_sql)
            .bind(&execution_id).fetch_one(db.pool()).await.unwrap();
        if mutation == "unchanged" {
            let resumed = result.expect("resume should succeed after the competing writer commits");
            assert_eq!(resumed.detail.step.status, "running");
            assert_eq!(after, (
                before.0 + 1, "running".into(), before.2 + 1, "running".into(),
                before.4 + 1, "running".into(), None, Some(runtime_state), before.8 + 1,
            ));
        } else {
            assert!(matches!(&result, Err(nomifun_db::DbError::Conflict(_))),
                "{mutation}: a changed target must return a CAS conflict, not a database lock error: {result:?}");
            assert_eq!(after, before, "{mutation}: conflict must roll back all partial resume writes and events");
        }
        db.close().await;
    }
}

#[tokio::test]
async fn recovered_input_handoff_waits_on_the_same_attempt_without_marking_work_complete() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, output) = user_input_fixture(&db, &repository, "ask_user").await;
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let lease = AgentExecutionLeaseToken::new("handoff-recovery".into());
    repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
    let recovered = repository.reconcile_recovered_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version, &output.attempt_id,
        running.current_attempt.as_ref().unwrap().attempt.version, &lease, Some(&output),
        &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    assert_eq!(recovered.disposition, AgentExecutionAttemptRecoveryDisposition::UserInputRequested);
    assert_eq!(recovered.detail.step.status, "waiting_input");
    let attempt = recovered.detail.current_attempt.unwrap().attempt;
    assert_eq!(attempt.status, "waiting_input");
    assert_eq!(attempt.question.as_deref(), Some("请提供姓名。"));
    assert!(attempt.finished_at.is_none());
    assert_eq!(repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap().status, "waiting_input");
    assert_eq!(repository.resolve_conversation_link(OWNER_ID, &output.conversation_id).await.unwrap().iter().filter(|link|link.active && link.relation=="attempt").count(),1);
    let events = repository.list_events(OWNER_ID,&execution_id,0,50).await.unwrap();
    let requested = events.last().unwrap();
    assert_eq!(requested.event_type,"decision_requested");
    let payload:serde_json::Value = serde_json::from_str(&requested.payload).unwrap();
    assert_eq!(payload["question"],"请提供姓名。");
    assert_eq!(payload["user_input_request"]["request_id"],"ask-name");
    let answer = nomifun_common::generate_id();
    let mut effects = nomifun_db::AttemptConversationEffects::default();
    effects.push_decision(answer.clone(),"小明".into()).unwrap();
    let execution=repository.get_execution(OWNER_ID,&execution_id).await.unwrap().unwrap();
    let resumed=repository.resume_waiting_attempt(OWNER_ID,&execution_id,execution.version,
        &recovered.detail.step.step_id,recovered.detail.step.version,&output.attempt_id,attempt.version,
        &nomifun_db::AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},
        &event(AgentExecutionEventKind::DecisionAnswered)).await.unwrap();
    let request=nomifun_agent_contracts::AgentUserInputRequest {
        request_id:"ask-class".into(),question:"请提供班级。".into(),remaining_work:"完成介绍。".into(),
    };
    let operation=format!("turn:user:{OWNER_ID}:{}:{answer}",output.conversation_id);
    let started=nomifun_common::generate_id();let terminal=nomifun_common::generate_id();
    for (seq,id,kind,value) in [(3,&started,"turn/started",serde_json::json!({})),
        (4,&terminal,"turn/completed",serde_json::json!({"user_input_request":request}))] {
        nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'answer-fixture',?,?,1,?,?)")
            .bind(&output.conversation_id).bind(seq).bind(id).bind(id).bind(kind).bind(&operation).bind(value.to_string()).execute(db.pool()).await.unwrap();
    }
    nomifun_db::sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,3,3,4)")
        .bind(&output.conversation_id).bind(&operation).bind(&operation).bind(&operation).bind(started).bind(&terminal).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_session_heads SET last_seq=4 WHERE session_id=?")
        .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
    let second=RecoveredAgentExecutionAttemptOutput {
        canonical_operation_id:operation,terminal_event_id:terminal,user_input_request:Some(request),..output
    };
    let waiting=repository.reconcile_recovered_attempt(OWNER_ID,&execution_id,&resumed.detail.step.step_id,resumed.detail.step.version,
        &second.attempt_id,resumed.detail.current_attempt.as_ref().unwrap().attempt.version,&lease,Some(&second),
        &NewAgentExecutionEvent {payload:serde_json::json!({"turn_operation_key":"caller-cannot-select-a-successor"}).to_string(),
            ..event(AgentExecutionEventKind::AttemptChanged)}).await.unwrap();
    let attempt=waiting.detail.current_attempt.unwrap().attempt;
    assert_eq!(attempt.question.as_deref(),Some("请提供班级。"));
    assert!(attempt.runtime_state.is_none(),"recovery retires the already-delivered answer instead of replaying its closed Turn");
    let events=repository.list_events(OWNER_ID,&execution_id,0,50).await.unwrap();
    let saved:serde_json::Value=serde_json::from_str(&events.last().unwrap().payload).unwrap();
    assert_eq!(saved["turn_operation_key"],answer,"the original verified Turn supplies its identity, not caller metadata");
    let mut tx=db.pool().begin().await.unwrap();
    assert_eq!(nomifun_db::execution_attempt_turn_key_tx(&mut tx,OWNER_ID,&execution_id,&waiting.detail.step.step_id,&attempt.attempt_id).await.unwrap(),answer,
        "readers share claim/resume's exact persisted continuation key after retiring the pending delivery");
    let reservations=nomifun_db::execution_reservations_tx(&mut tx,&execution_id).await.unwrap();
    assert_eq!(reservations.len(),1);
    assert!(!reservations[0].capacity_held,
        "a second typed question uses its original saved continuation identity after the delivery queue retires");
    tx.commit().await.unwrap();
    let mut pending=nomifun_db::AttemptConversationEffects::decode(attempt.runtime_state.as_deref()).unwrap();
    let next=nomifun_common::generate_id();pending.push_decision(next.clone(),"一年级".into()).unwrap();
    assert_eq!(pending.current_turn_operation_key(&attempt.attempt_id).unwrap(),next);
    let execution=repository.get_execution(OWNER_ID,&execution_id).await.unwrap().unwrap();
    let resumed=repository.resume_waiting_attempt(OWNER_ID,&execution_id,execution.version,
        &waiting.detail.step.step_id,waiting.detail.step.version,&attempt.attempt_id,attempt.version,
        &nomifun_db::AttemptConversationEffectParams {runtime_state:Some(pending.encode().unwrap())},
        &event(AgentExecutionEventKind::DecisionAnswered)).await.unwrap();
    assert_eq!(resumed.detail.step.status,"running");
}

async fn user_input_fixture(
    db: &nomifun_db::Database, repository: &SqliteAgentExecutionRepository, policy: &str,
) -> (String, nomifun_db::AgentExecutionStepDetailRow, RecoveredAgentExecutionAttemptOutput) {
    let (execution_id, running, mut output) = recovery_fixture(db, repository).await;
    let request = nomifun_agent_contracts::AgentUserInputRequest {
        request_id:"ask-name".into(),question:"请提供姓名。".into(),remaining_work:"收到资料后生成 PPT。".into(),
    };
    let started = nomifun_common::generate_id();
    nomifun_db::sqlx::query("UPDATE agent_events SET seq=2 WHERE event_id=?").bind(&output.terminal_event_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'handoff-fixture',?,'turn/started',1,?,'{}')")
        .bind(&output.conversation_id).bind(&started).bind(&started).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    let terminal = serde_json::json!({"user_input_request":request}).to_string();
    nomifun_db::sqlx::query("UPDATE agent_events SET causation_event_id=?,inline_json=? WHERE event_id=?")
        .bind(&started).bind(&terminal).bind(&output.terminal_event_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_turns SET started_event_id=? WHERE session_id=? AND operation_id=?")
        .bind(started).bind(&output.conversation_id).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,last_seq,active_set_generation,unread_count) VALUES(?,'ready',2,0,0)")
        .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_executions SET decision_policy=? WHERE execution_id=?")
        .bind(policy).bind(&execution_id).execute(db.pool()).await.unwrap();
    output.user_input_request = Some(request);
    output.disposition = nomifun_db::AgentExecutionAttemptDisposition::InputRequired;
    output.text = Some("请提供姓名后继续。".into());
    (execution_id, running, output)
}

#[tokio::test]
async fn completed_input_source_guard_rejects_identity_content_and_successor_drift() {
    for mutation in ["content", "terminal", "successor", "link"] {
        let db = database().await;
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution_id, running, output) = user_input_fixture(&db,&repository,if mutation=="automatic" {"automatic"} else {"ask_user"}).await;
        let mut source = nomifun_db::AgentExecutionTerminalTurnGuard::InputRequired {
            conversation_id:output.conversation_id.clone(),canonical_operation_id:output.canonical_operation_id.clone(),
            terminal_event_id:output.terminal_event_id.clone(),user_input_request:output.user_input_request.clone().unwrap(),
        };
        if mutation=="content" { if let nomifun_db::AgentExecutionTerminalTurnGuard::InputRequired { user_input_request, .. } = &mut source { user_input_request.remaining_work="changed scope".into(); } }
        if mutation=="terminal" { if let nomifun_db::AgentExecutionTerminalTurnGuard::InputRequired { terminal_event_id, .. } = &mut source { *terminal_event_id=nomifun_common::generate_id(); } }
        if mutation=="successor" {
            nomifun_db::sqlx::query("UPDATE agent_session_heads SET active_turn_id='successor',status='running' WHERE session_id=?")
                .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        }
        if mutation=="link" {
            nomifun_db::sqlx::query("UPDATE conversation_execution_links SET active=0 WHERE conversation_id=?")
                .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        }
        let result=repository.settle_attempt(OWNER_ID,&execution_id,&running.step.step_id,running.step.version,
            &output.attempt_id,running.current_attempt.as_ref().unwrap().attempt.version,None,
            &nomifun_db::SettleAgentExecutionAttemptParams {
                expected_active_session_turn:None,expected_terminal_turn:Some(source),
                attempt_status:nomifun_common::ExecutionAttemptStatus::WaitingInput,
                step_status:ExecutionStepStatus::WaitingInput,execution_status:Some(AgentExecutionStatus::WaitingInput),
                question:Some(Some("请提供姓名。".into())),error:None,output_summary:None,output_files:None,
                tokens:None,retry_after:None,runtime_state:None,started_at:None,finished_at:None,loop_repeat_reset:None,
            },&event(AgentExecutionEventKind::DecisionRequested)).await;
        assert!(result.is_err(),"{mutation} must not confer waiting-input authority");
        assert_eq!(repository.get_step_detail(OWNER_ID,&execution_id,&running.step.step_id).await.unwrap().unwrap().step.status,"running");
    }
}

#[tokio::test]
async fn recovery_adopts_exact_typed_output_and_never_session_control_metadata() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, output) = recovery_fixture(&db, &repository).await;
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let lease = AgentExecutionLeaseToken::new("recovery-owner".to_owned());
    repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms() + 30_000)
        .await.unwrap().unwrap();
    let recovered = repository.reconcile_recovered_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version,
        &lease, Some(&output), &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    assert_eq!(recovered.disposition, AgentExecutionAttemptRecoveryDisposition::CompletedReceiptAdopted);
    let attempt = recovered.detail.current_attempt.unwrap().attempt;
    assert_eq!(attempt.status, "completed");
    assert_eq!(attempt.output_summary.as_deref(), Some("已完成游戏实现"));
    assert_eq!(serde_json::from_str::<Vec<String>>(&attempt.output_files).unwrap(), output.output_files);
    assert_eq!(attempt.tokens, Some(12));
    assert!(!attempt.output_summary.unwrap().contains("model_steps"));
}

#[tokio::test]
async fn recovery_rejects_mismatched_terminal_identity_and_empty_public_delivery() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, mut output) = recovery_fixture(&db, &repository).await;
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let lease = AgentExecutionLeaseToken::new("recovery-owner".to_owned());
    repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms() + 30_000)
        .await.unwrap().unwrap();
    let actual_terminal = output.terminal_event_id.clone();
    output.terminal_event_id = nomifun_common::generate_id();
    assert!(repository.reconcile_recovered_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version,
        &lease, Some(&output), &event(AgentExecutionEventKind::AttemptChanged),
    ).await.is_err());
    output.terminal_event_id = actual_terminal;
    output.text = Some(" \n".to_owned());
    output.output_files.clear();
    let recovered = repository.reconcile_recovered_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version,
        &lease, Some(&output), &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    assert_eq!(recovered.detail.step.status, "failed");
    assert_eq!(recovered.detail.current_attempt.unwrap().attempt.status, "failed");
}

#[tokio::test]
async fn dispatch_pause_preserves_the_exact_running_attempt_and_pending_controls() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, _) = recovery_fixture(&db, &repository).await;
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let attempt = &running.current_attempt.as_ref().unwrap().attempt;
    repository.enqueue_attempt_conversation_effect(
        OWNER_ID, &execution_id, execution.version, &running.step.step_id, running.step.version,
        &attempt.attempt_id, attempt.version,
        &nomifun_db::AttemptConversationEffectParams { runtime_state: Some(serde_json::json!({
            "pending_conversation_effects": [{"kind":"steer","operation_id":"pending-correction","target_operation_id":"target-turn","content":"preserve this correction"}]
        }).to_string()) }, &event(AgentExecutionEventKind::StepChanged),
    ).await.unwrap();
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let lease = AgentExecutionLeaseToken::new("dispatch-pause-owner".into());
    let expires_at = nomifun_common::now_ms() + 30_000;
    repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), expires_at).await.unwrap().unwrap();
    let before = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id).await.unwrap().unwrap();
    let paused_execution = repository.pause_execution(OWNER_ID, &execution_id, execution.version, &event(AgentExecutionEventKind::StatusChanged))
        .await.unwrap();
    let paused = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id)
        .await.unwrap().unwrap();
    assert_eq!(paused_execution.status, "paused");
    assert_eq!(paused_execution.lease_owner.as_deref(), Some(lease.owner()));
    assert_eq!(paused.step.status, "running");
    assert_eq!(paused.step.version, before.step.version);
    let attempt = paused.current_attempt.unwrap().attempt;
    let before_attempt = before.current_attempt.unwrap().attempt;
    assert_eq!(attempt.attempt_id, before_attempt.attempt_id);
    assert_eq!(attempt.version, before_attempt.version);
    assert_eq!(attempt.runtime_state, before_attempt.runtime_state);
    assert!(attempt.output_summary.is_none());
    let state = attempt.runtime_state.unwrap();
    assert!(!state.contains("review_blocked"), "a dispatch pause cannot quarantine a healthy original owner");
    assert!(state.contains("preserve this correction"));
    assert!(repository.list_conversation_links(OWNER_ID, &execution_id).await.unwrap().iter()
        .any(|link| link.relation == "attempt" && link.active && link.cleanup_completed_at.is_none()));
    assert!(repository.list_pending_conversation_cleanups(Some(&execution_id), None, 100).await.unwrap().cleanups.is_empty());
    let resumed = repository.resume_execution(OWNER_ID, &execution_id, paused_execution.version,
        &event(AgentExecutionEventKind::StatusChanged)).await.unwrap();
    assert_eq!(resumed.status, "running");
    assert_eq!(resumed.lease_owner.as_deref(), Some(lease.owner()), "resume cannot fence its still-running original owner");
}

#[tokio::test]
async fn recovery_uses_the_current_decision_turn_instead_of_the_closed_initial_turn() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, initial_output) = recovery_fixture(&db, &repository).await;
    let attempt = &running.current_attempt.as_ref().unwrap().attempt;
    let waiting = repository.settle_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &attempt.attempt_id, attempt.version, None,
        &nomifun_db::SettleAgentExecutionAttemptParams {
            expected_terminal_turn: None,
            expected_active_session_turn: None,
            attempt_status: nomifun_common::ExecutionAttemptStatus::WaitingInput,
            step_status: ExecutionStepStatus::WaitingInput,
            execution_status: Some(AgentExecutionStatus::WaitingInput),
            question: Some(Some("continue?".to_owned())), error: None,
            output_summary: None, output_files: None, tokens: None, retry_after: None,
            runtime_state: Some(None), started_at: None, finished_at: None, loop_repeat_reset: None,
        }, &event(AgentExecutionEventKind::DecisionRequested),
    ).await.unwrap();
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let mut effects = nomifun_db::AttemptConversationEffects::default();
    effects.push_decision("decision-current".into(), "continue".into()).unwrap();
    repository.resume_waiting_attempt(
        OWNER_ID, &execution_id, execution.version, &waiting.step.step_id, waiting.step.version,
        &attempt.attempt_id, waiting.current_attempt.as_ref().unwrap().attempt.version,
        &nomifun_db::AttemptConversationEffectParams { runtime_state: Some(effects.encode().unwrap()) },
        &event(AgentExecutionEventKind::DecisionAnswered),
    ).await.unwrap();
    let operation_id = format!("turn:user:{OWNER_ID}:{}:decision-current", initial_output.conversation_id);
    let terminal_event_id = nomifun_common::generate_id();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_events (session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) \
         VALUES (?,2,?,'recovery-fixture',?,'turn/completed',1,?,'{}')"
    ).bind(&initial_output.conversation_id).bind(&terminal_event_id).bind(&terminal_event_id).bind(&operation_id)
        .execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_turns (session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,started_at,finished_at) \
         VALUES (?,?,?,?,'completed',?,2,2,2)"
    ).bind(&initial_output.conversation_id).bind(&operation_id).bind(&operation_id).bind(&operation_id)
        .bind(&terminal_event_id).execute(db.pool()).await.unwrap();
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let current = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id).await.unwrap().unwrap();
    let lease = AgentExecutionLeaseToken::new("current-decision-owner".into());
    repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
    let current_attempt = current.current_attempt.as_ref().unwrap();
    assert!(repository.reconcile_recovered_attempt(
        OWNER_ID, &execution_id, &current.step.step_id, current.step.version,
        &current_attempt.attempt.attempt_id, current_attempt.attempt.version, &lease,
        Some(&initial_output), &event(AgentExecutionEventKind::AttemptChanged),
    ).await.is_err(), "a completed older Turn is not the resumed invocation's output");
    let current_output = RecoveredAgentExecutionAttemptOutput {
        canonical_operation_id: operation_id, terminal_event_id,
        text: Some("current continuation output".to_owned()),
        ..initial_output
    };
    repository.reconcile_recovered_attempt(
        OWNER_ID, &execution_id, &current.step.step_id, current.step.version,
        &current_attempt.attempt.attempt_id, current_attempt.attempt.version, &lease,
        Some(&current_output), &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    let paused = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id).await.unwrap().unwrap();
    assert_eq!(paused.step.status, "completed");
    assert_eq!(paused.current_attempt.unwrap().attempt.output_summary.as_deref(), Some("current continuation output"));
}

#[tokio::test]
async fn native_decision_request_is_atomically_bound_to_the_calling_turn() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, original) = recovery_fixture(&db, &repository).await;
    let attempt = &running.current_attempt.as_ref().unwrap().attempt;
    let successor = format!("turn:user:{OWNER_ID}:{}:decision-next", original.conversation_id);
    nomifun_db::sqlx::query(
        "INSERT INTO agent_turns (session_id,turn_id,operation_id,idempotency_key,state,accepted_at,started_at) \
         VALUES (?,?,?,?,'running',2,2)",
    ).bind(&original.conversation_id).bind(&successor).bind(&successor).bind(&successor)
        .execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_session_heads (session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) \
         VALUES (?,'running',?,1,2,0)",
    ).bind(&original.conversation_id).bind(&successor).execute(db.pool()).await.unwrap();
    let mut params = nomifun_db::SettleAgentExecutionAttemptParams {
        expected_terminal_turn: None,
        expected_active_session_turn: Some(nomifun_db::AgentExecutionActiveTurnGuard {
            conversation_id: original.conversation_id.clone(),
            canonical_operation_id: original.canonical_operation_id.clone(),
        }),
        attempt_status: nomifun_common::ExecutionAttemptStatus::WaitingInput,
        step_status: ExecutionStepStatus::WaitingInput,
        execution_status: Some(AgentExecutionStatus::WaitingInput), question: Some(Some("old question".into())),
        error: None, output_summary: None, output_files: None, tokens: None, retry_after: None,
        runtime_state: None, started_at: None, finished_at: None, loop_repeat_reset: None,
    };
    let request = event(AgentExecutionEventKind::DecisionRequested);
    assert!(matches!(repository.settle_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &attempt.attempt_id, attempt.version, None, &params, &request,
    ).await, Err(nomifun_db::DbError::Conflict(_))));
    let unchanged = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id).await.unwrap().unwrap();
    assert_eq!(unchanged.step.version, running.step.version);
    assert_eq!(unchanged.step.status, "running");
    assert_eq!(unchanged.current_attempt.as_ref().unwrap().attempt.version, attempt.version);
    params.expected_active_session_turn.as_mut().unwrap().canonical_operation_id = successor.clone();
    let requested = repository.settle_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &attempt.attempt_id, attempt.version, None, &params, &request,
    ).await.unwrap();
    assert_eq!(requested.step.status, "waiting_input");
    assert!(requested.step.version > running.step.version);
    let active: String = nomifun_db::sqlx::query_scalar("SELECT active_turn_id FROM agent_session_heads WHERE session_id=?")
        .bind(&original.conversation_id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(active, successor, "domain WaitingInput does not cancel or replace a different canonical Turn");
}

async fn cancelled_terminal_fixture(db: &nomifun_db::Database, repository: &SqliteAgentExecutionRepository)
    -> (String, nomifun_db::AgentExecutionStepDetailRow, RecoveredAgentExecutionAttemptOutput) {
    let (execution_id, running, mut output) = user_input_fixture(db, repository, "automatic").await;
    nomifun_db::sqlx::query("UPDATE agent_turns SET state='cancelled' WHERE session_id=? AND operation_id=?")
        .bind(&output.conversation_id).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_events SET kind='turn/cancelled',inline_json='{}' WHERE event_id=?")
        .bind(&output.terminal_event_id).execute(db.pool()).await.unwrap();
    output.disposition = nomifun_db::AgentExecutionAttemptDisposition::Cancelled;
    output.user_input_request = None;
    output.error = None;
    output.text = None;
    output.output_files.clear();
    (execution_id, running, output)
}

#[tokio::test]
async fn automatic_human_handoff_keeps_the_exact_attempt_waiting_until_a_real_reply() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, output) = user_input_fixture(&db, &repository, "automatic").await;
    let request = output.user_input_request.clone().unwrap();
    let source = nomifun_db::AgentExecutionTerminalTurnGuard::InputRequired {
        conversation_id: output.conversation_id.clone(), canonical_operation_id: output.canonical_operation_id.clone(),
        terminal_event_id: output.terminal_event_id.clone(), user_input_request: request.clone(),
    };
    let waiting = repository.settle_attempt(OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version, None,
        &nomifun_db::SettleAgentExecutionAttemptParams {
            expected_active_session_turn: None, expected_terminal_turn: Some(source),
            attempt_status:nomifun_common::ExecutionAttemptStatus::WaitingInput, step_status:ExecutionStepStatus::WaitingInput,
            execution_status:Some(AgentExecutionStatus::WaitingInput), question:Some(Some(request.question)), error:Some(None),
            output_summary:None, output_files:None, tokens:None, retry_after:Some(None), runtime_state:Some(None),
            started_at:None, finished_at:None, loop_repeat_reset:None,
        }, &event(AgentExecutionEventKind::DecisionRequested)).await.unwrap();
    let attempt = waiting.current_attempt.unwrap().attempt;
    assert_eq!(attempt.attempt_id, output.attempt_id);
    assert_eq!(attempt.status, "waiting_input");
    assert!(attempt.finished_at.is_none() && attempt.error.is_none() && attempt.retry_after.is_none());
    assert_eq!(repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap().status, "waiting_input");
    let links = repository.resolve_conversation_link(OWNER_ID, &output.conversation_id).await.unwrap();
    assert_eq!(links.iter().filter(|link| link.active && link.attempt_id.as_deref() == Some(output.attempt_id.as_str())).count(), 1);
}

#[tokio::test]
async fn cancelled_canonical_terminal_settles_only_its_task_and_rejects_foreign_source() {
    for mutation in ["terminal", "kind", "successor", "link", "unknown_effect", "valid"] {
        let db = database().await;
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution_id, running, output) = cancelled_terminal_fixture(&db, &repository).await;
        match mutation {
            "kind" => { nomifun_db::sqlx::query("UPDATE agent_events SET kind='turn/completed' WHERE event_id=?")
                .bind(&output.terminal_event_id).execute(db.pool()).await.unwrap(); },
            "successor" => { nomifun_db::sqlx::query("UPDATE agent_session_heads SET status='running',active_turn_id='different-turn' WHERE session_id=?")
                .bind(&output.conversation_id).execute(db.pool()).await.unwrap(); },
            "link" => { nomifun_db::sqlx::query("UPDATE conversation_execution_links SET active=0 WHERE conversation_id=?")
                .bind(&output.conversation_id).execute(db.pool()).await.unwrap(); },
            "unknown_effect" => {
                let started: String = nomifun_db::sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
                    .bind(&output.conversation_id).bind(&output.canonical_operation_id).fetch_one(db.pool()).await.unwrap();
                nomifun_db::sqlx::query("INSERT INTO agent_effects(effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id,input_digest,strategy,state,started_event_id,created_at) VALUES(?,?,?,?,'workspace','workspace.process','workspace.process/start',?,'managed_effect','pending',?,1)")
                    .bind(nomifun_common::generate_id()).bind(&output.conversation_id).bind(&output.canonical_operation_id)
                    .bind(&output.canonical_operation_id).bind("a".repeat(64)).bind(started).execute(db.pool()).await.unwrap();
            },
            _ => {},
        }
        let result = repository.settle_attempt(OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
            &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version, None,
            &nomifun_db::SettleAgentExecutionAttemptParams {
                expected_active_session_turn:None, expected_terminal_turn:Some(nomifun_db::AgentExecutionTerminalTurnGuard::Cancelled {
                    conversation_id:output.conversation_id.clone(), canonical_operation_id:output.canonical_operation_id.clone(),
                    terminal_event_id:if mutation=="terminal" {nomifun_common::generate_id()} else {output.terminal_event_id.clone()},
                }),
                attempt_status:nomifun_common::ExecutionAttemptStatus::Cancelled, step_status:ExecutionStepStatus::Cancelled,
                execution_status:None, question:Some(None),error:Some(None),output_summary:None,
                output_files:None,tokens:None,retry_after:Some(None),runtime_state:Some(None),started_at:None,finished_at:None,loop_repeat_reset:None,
            }, &event(AgentExecutionEventKind::AttemptChanged)).await;
        if mutation!="valid" {
            assert!(result.is_err());
            assert_eq!(repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap().status, "running");
        } else {
            let cancelled = result.unwrap();
            assert_eq!(cancelled.step.status, "cancelled");
            let attempt = cancelled.current_attempt.unwrap().attempt;
            assert_eq!(attempt.status, "cancelled");
            assert!(attempt.error.is_none() && attempt.retry_after.is_none());
            let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
            assert_eq!(execution.status, "running");
            assert!(execution.lease_owner.is_none());
            assert!(repository.resolve_conversation_link(OWNER_ID, &output.conversation_id).await.unwrap().iter().all(|link|!link.active));
        }
    }
}

#[tokio::test]
async fn recovery_adopts_canonical_cancellation_without_cancelling_a_dispatch_paused_aggregate() {
    for during_pause in [false, true] {
        let db = database().await;
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution_id, running, output) = cancelled_terminal_fixture(&db, &repository).await;
        let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
        let lease = AgentExecutionLeaseToken::new("cancel-recovery-owner".into());
        repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
        if during_pause {
            let result = repository.pause_execution(OWNER_ID, &execution_id, execution.version,
                &event(AgentExecutionEventKind::StatusChanged)).await.unwrap();
            assert_eq!(result.status, "paused");
        }
        let result = repository.reconcile_recovered_attempt(OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
                &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version, &lease, Some(&output),
                &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        assert_eq!(result.disposition, AgentExecutionAttemptRecoveryDisposition::CancelledReceiptAdopted);
        let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
        assert_eq!(execution.status, if during_pause { "paused" } else { "running" });
        assert_eq!(execution.lease_owner.as_deref(), Some(lease.owner()));
        let detail = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id).await.unwrap().unwrap();
        assert_eq!(detail.step.status, "cancelled");
        let attempt = detail.current_attempt.unwrap().attempt;
        assert_eq!(attempt.attempt_id, output.attempt_id);
        assert_eq!(attempt.status, "cancelled");
        assert!(attempt.error.is_none() && attempt.retry_after.is_none());
    }
}

#[path = "agent_execution_repository/scoped_stop.rs"]
mod scoped_stop;

#[path = "agent_execution_repository/cleanup_exact.rs"]
mod cleanup_exact;

#[path = "agent_execution_repository/cleanup_continuation.rs"]
mod cleanup_continuation;

#[path = "agent_execution_repository/automation_stop_admission.rs"]
mod automation_stop_admission;

#[path = "agent_execution_repository/recovery_handoff.rs"]
mod recovery_handoff;

#[path = "agent_execution_repository/capacity_reservations.rs"]
mod capacity_reservations;

#[path = "agent_execution_repository/delegation_creation.rs"]
mod delegation_creation;

#[path = "agent_execution_repository/source_continuation.rs"]
mod source_continuation;

#[path = "agent_execution_repository/source_reply.rs"]
mod source_reply;

#[path = "agent_execution_repository/parent_question_reply.rs"]
mod parent_question_reply;

#[path = "agent_execution_repository/task_control_reliability.rs"]
mod task_control_reliability;

#[path = "agent_execution_repository/closed_steer.rs"]
mod closed_steer;

#[path = "agent_execution_repository/task_retirement.rs"]
mod task_retirement;

#[path = "agent_execution_repository/stopped_backoff_cleanup.rs"]
mod stopped_backoff_cleanup;

#[path = "agent_execution_repository/delegation_records.rs"]
mod delegation_records;
