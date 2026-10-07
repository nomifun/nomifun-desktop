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
async fn autowork_attempt_reuses_lead_without_becoming_disposable_attempt_transcript() {
    let db = database().await;
    insert_canonical_session(&db, CANONICAL_LEAD_ID, OWNER_ID, "live").await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let participant_id = nomifun_common::generate_id();
    let mut params = execution_params();
    params.lead_conversation_id = Some(CANONICAL_LEAD_ID.to_owned());
    params.initial_plan_input = serde_json::json!({
        "mode": "automation",
        "source": {
            "requirement_id": "0190f5fe-7c00-7a00-8000-000000000191",
            "claim_generation": 1,
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
            None,
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
    let participant_id = nomifun_common::generate_id();
    repository
        .create_execution_with_participants(
            OWNER_ID,
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
    let legacy_plan_gate: String = nomifun_db::sqlx::query_scalar(
        "SELECT plan_gate FROM agent_executions WHERE execution_id = ?",
    )
    .bind(&created.execution_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(legacy_plan_gate, "automatic");
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
            None,
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
    let created = create_execution(repository).await;
    let participant_id = repository.get_execution_detail(OWNER_ID, &created.execution_id)
        .await.unwrap().unwrap().participants[0].participant_id.clone();
    let planned = repository.reconcile_plan(
        OWNER_ID, &created.execution_id, created.version,
        &ReconcileAgentExecutionPlanParams {
            goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None,
            keep_step_ids: Vec::new(), new_participants: Vec::new(), retire_participant_ids: Vec::new(),
            new_steps: vec![step(nomifun_common::generate_id(), Some(participant_id.clone()), "recovery")],
            new_dependencies: Vec::new(), execution_status: AgentExecutionStatus::Running,
        }, &event(AgentExecutionEventKind::PlanChanged),
    ).await.unwrap();
    let queued = repository.create_attempt(
        OWNER_ID, &created.execution_id, &planned.steps[0].step_id, planned.steps[0].version,
        None, &CreateAgentExecutionAttemptParams {
            participant_id: Some(participant_id), start_immediately: false,
            trigger_reason: "initial".to_owned(), effective_config: "{}".to_owned(),
            retry_after: None, runtime_state: None,
        }, &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    let attempt = queued.current_attempt.as_ref().unwrap().attempt.clone();
    let conversation_id = nomifun_common::generate_id();
    insert_canonical_session(db, &conversation_id, OWNER_ID, "live").await;
    let running = repository.start_attempt(
        OWNER_ID, &created.execution_id, &queued.step.step_id, queued.step.version,
        &attempt.attempt_id, attempt.version, &conversation_id,
        AgentExecutionAttemptSessionKind::ChildAttempt, None, &event(AgentExecutionEventKind::AttemptChanged),
    ).await.unwrap();
    let operation_id = format!("turn:user:{OWNER_ID}:{conversation_id}:{}:initial-turn", attempt.attempt_id);
    let terminal_event_id = nomifun_common::generate_id();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_events (session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) \
         VALUES (?,1,?,'recovery-fixture',?,'turn/completed',1,?,?)"
    ).bind(&conversation_id).bind(&terminal_event_id).bind(&terminal_event_id).bind(&operation_id)
        .bind(r#"{"model_steps":3,"finish_reason":"stop","finished_at_ms":10}"#)
        .execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_turns (session_id,turn_id,operation_id,idempotency_key,state,result_json,terminal_event_id,accepted_at,started_at,finished_at) \
         VALUES (?,?,?,?,'completed',?,?,1,1,1)"
    ).bind(&conversation_id).bind(&operation_id).bind(&operation_id).bind(&operation_id)
        .bind(r#"{"model_steps":3,"finish_reason":"stop","finished_at_ms":10}"#)
        .bind(&terminal_event_id).execute(db.pool()).await.unwrap();
    (created.execution_id, running, RecoveredAgentExecutionAttemptOutput {
        attempt_id: attempt.attempt_id, conversation_id, canonical_operation_id: operation_id,
        terminal_event_id, ok: true, text: Some("已完成游戏实现".to_owned()),
        output_files: vec!["/workspace/index.html".to_owned()], error: None, tokens: Some(12),
    })
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
async fn recovery_pause_requires_typed_output_before_adopting_completed_turn() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, output) = recovery_fixture(&db, &repository).await;
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    repository.pause_execution(OWNER_ID, &execution_id, execution.version, &[output], &event(AgentExecutionEventKind::StatusChanged))
        .await.unwrap();
    let paused = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id)
        .await.unwrap().unwrap();
    assert_eq!(paused.step.status, "completed");
    assert_eq!(paused.current_attempt.unwrap().attempt.output_summary.as_deref(), Some("已完成游戏实现"));

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
    repository.pause_execution(OWNER_ID, &execution_id, execution.version, &[], &event(AgentExecutionEventKind::StatusChanged))
        .await.unwrap();
    let paused = repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id)
        .await.unwrap().unwrap();
    assert_eq!(paused.step.status, "waiting_input");
    let attempt = paused.current_attempt.unwrap().attempt;
    assert!(attempt.output_summary.is_none());
    let state = attempt.runtime_state.unwrap();
    assert!(state.contains("terminal_output_unavailable"));
    assert!(state.contains("preserve this correction"), "manual review must retain accepted but undelivered input");
}

#[tokio::test]
async fn recovery_pause_uses_the_current_decision_turn_instead_of_the_closed_initial_turn() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution_id, running, initial_output) = recovery_fixture(&db, &repository).await;
    let attempt = &running.current_attempt.as_ref().unwrap().attempt;
    let waiting = repository.settle_attempt(
        OWNER_ID, &execution_id, &running.step.step_id, running.step.version,
        &attempt.attempt_id, attempt.version, None,
        &nomifun_db::SettleAgentExecutionAttemptParams {
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
        "INSERT INTO agent_turns (session_id,turn_id,operation_id,idempotency_key,state,result_json,terminal_event_id,accepted_at,started_at,finished_at) \
         VALUES (?,?,?,?,'completed','{}',?,2,2,2)"
    ).bind(&initial_output.conversation_id).bind(&operation_id).bind(&operation_id).bind(&operation_id)
        .bind(&terminal_event_id).execute(db.pool()).await.unwrap();
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    assert!(repository.pause_execution(
        OWNER_ID, &execution_id, execution.version, &[initial_output.clone()], &event(AgentExecutionEventKind::StatusChanged),
    ).await.is_err(), "a completed older Turn is not the resumed invocation's output");
    let current_output = RecoveredAgentExecutionAttemptOutput {
        canonical_operation_id: operation_id, terminal_event_id,
        text: Some("current continuation output".to_owned()),
        ..initial_output
    };
    repository.pause_execution(
        OWNER_ID, &execution_id, execution.version, &[current_output], &event(AgentExecutionEventKind::StatusChanged),
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
