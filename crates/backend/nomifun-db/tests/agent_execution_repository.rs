use nomifun_db::models::ConversationRow;
use nomifun_db::{
    AdoptAgentExecutionStepOutputParams, AgentExecutionLeaseToken,
    AppendAgentExecutionStepsFromAttemptParams, AppendAgentExecutionStepsParams,
    AttemptConversationEffectParams, CreateAgentExecutionAttemptParams,
    CreateAgentExecutionParams, IAgentExecutionRepository, IConversationRepository,
    LoopRepeatResetParams, NewAgentExecutionEvent, NewAgentExecutionParticipant,
    NewAgentExecutionStep,
    NewAgentExecutionStepDependency, ReconcileAgentExecutionPlanParams,
    RetryAgentExecutionStep, SettleAgentExecutionAttemptParams, SqliteAgentExecutionRepository,
    SqliteConversationRepository, UpdateAgentExecutionParams, init_database_memory,
};
use nomifun_common::{
    AdaptationPolicy, AgentExecutionEventKind, AgentExecutionStatus, AgentStepMode,
    AgentToolPolicy,
    DecisionPolicy, DelegationPolicy, ExecutionAttemptStatus, ExecutionStepKind,
    ExecutionStepStatus, ParticipantAssignmentSource, PlanGate, StepFailurePolicy,
};

const USER_ID: &str = "system_default_user";

async fn test_database() -> nomifun_db::Database {
    let database = init_database_memory().await.unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO providers (\
            id, platform, name, base_url, api_key_encrypted, models, enabled, \
            capabilities, created_at, updated_at\
         ) VALUES ('provider_test', 'openai', 'test', 'https://example.invalid', \
                   'encrypted', '[\"model_test\"]', 1, '[]', 1, 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    database
}

fn event(event_type: AgentExecutionEventKind) -> NewAgentExecutionEvent {
    NewAgentExecutionEvent {
        event_type,
        step_id: None,
        attempt_id: None,
        actor: nomifun_common::AgentExecutionActor::system(),
        payload: "{}".to_string(),
    }
}

fn participant(id: &str) -> NewAgentExecutionParticipant {
    NewAgentExecutionParticipant {
        id: id.to_string(),
        source_agent_id: "agent_nomi".to_string(),
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
        provider_id: Some("provider_test".to_string()),
        model: Some("model_test".to_string()),
        role: Some("builder".to_string()),
        capability: Some(r#"{"coding":true}"#.to_string()),
        constraints: Some("{}".to_string()),
        description: None,
        system_prompt: None,
        enabled_skills: "[]".to_string(),
        disabled_builtin_skills: "[]".to_string(),
        sort_order: 0,
    }
}

fn agent_step(id: &str, participant_id: &str) -> NewAgentExecutionStep {
    NewAgentExecutionStep {
        id: id.to_string(),
        title: format!("step {id}"),
        spec: format!("execute {id}"),
        role: Some("builder".to_string()),
        tool_policy: AgentToolPolicy::Full,
        kind: ExecutionStepKind::Agent,
        agent_mode: Some(AgentStepMode::Normal),
        profile: Some("{}".to_string()),
        fanout_group: None,
        control_policy: None,
        status: ExecutionStepStatus::Pending,
        assigned_participant_id: Some(participant_id.to_string()),
        assignment_score: Some(1.0),
        assignment_rationale: Some("test".to_string()),
        assignment_source: Some(ParticipantAssignmentSource::Planner),
        assignment_locked: false,
        failure_policy: StepFailurePolicy::FailExecution,
        preset_prompt: None,
        graph_x: None,
        graph_y: None,
    }
}

fn loop_step(id: &str) -> NewAgentExecutionStep {
    NewAgentExecutionStep {
        id: id.to_string(),
        title: format!("step {id}"),
        spec: format!("repeat {id}"),
        role: None,
        tool_policy: AgentToolPolicy::Full,
        kind: ExecutionStepKind::Loop,
        agent_mode: None,
        profile: None,
        fanout_group: None,
        control_policy: Some(
            r#"{"kind":"loop","max_iterations":2,"stop":{"kind":"max_iterations"}}"#
                .to_string(),
        ),
        status: ExecutionStepStatus::Pending,
        assigned_participant_id: None,
        assignment_score: None,
        assignment_rationale: None,
        assignment_source: None,
        assignment_locked: false,
        failure_policy: StepFailurePolicy::FailExecution,
        preset_prompt: None,
        graph_x: None,
        graph_y: None,
    }
}

async fn create_conversation(repo: &SqliteConversationRepository, name: &str) -> i64 {
    let now = nomifun_common::now_ms();
    repo.create(&ConversationRow {
        id: 0,
        user_id: USER_ID.to_string(),
        name: name.to_string(),
        r#type: "nomi".to_string(),
        extra: "{}".to_string(),
        delegation_policy: "automatic".to_string(),
        execution_model_pool: None,
        decision_policy: "automatic".to_string(),
        execution_template_id: None,
        model: None,
        status: Some("pending".to_string()),
        source: Some("nomifun".to_string()),
        channel_chat_id: None,
        pinned: false,
        pinned_at: None,
        cron_job_id: None,
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap()
}

async fn create_execution(
    repository: &SqliteAgentExecutionRepository,
    lead_conversation_id: i64,
) -> String {
    repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "test execution".to_string(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 4,
                work_dir: None,
                lead_conversation_id: Some(lead_conversation_id),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("participant_1")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn lead_conversation_has_at_most_one_unfinished_execution() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead = create_conversation(&conversations, "shared lead").await;
    let first_id = create_execution(&repository, lead).await;

    let duplicate = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "conflicting execution".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: Some(lead),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("participant_2")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap_err();
    assert!(duplicate.to_string().contains("unfinished Agent Execution"));

    repository
        .cancel_execution(USER_ID, &first_id, 0, &event(AgentExecutionEventKind::StatusChanged))
        .await
        .unwrap();

    let replacement = create_execution(&repository, lead).await;
    assert_ne!(replacement, first_id);
    let links = repository
        .resolve_conversation_link(USER_ID, lead)
        .await
        .unwrap();
    assert_eq!(
        links
            .iter()
            .filter(|link| link.active && link.relation == "lead")
            .count(),
        1,
        "a Conversation has exactly one current lead Execution"
    );
    assert_eq!(
        links
            .iter()
            .find(|link| link.active && link.relation == "lead")
            .map(|link| link.execution_id.as_str()),
        Some(replacement.as_str())
    );
    assert!(links.iter().any(|link| {
        !link.active && link.relation == "lead" && link.execution_id == first_id
    }));
}

#[tokio::test]
async fn schema_guards_owner_authority_idempotency_and_terminal_reopen_boundaries() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let now = nomifun_common::now_ms();
    nomifun_db::sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_2', 'user_2', 'hash', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();
    let foreign_conversation: i64 = nomifun_db::sqlx::query_scalar(
        "INSERT INTO conversations \
         (user_id, name, type, extra, delegation_policy, status, created_at, updated_at) \
         VALUES ('user_2', 'foreign', 'nomi', '{}', 'disabled', 'pending', ?, ?) RETURNING id",
    )
    .bind(now)
    .bind(now)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let lead = create_conversation(&conversations, "reopen lead").await;
    let first_id = create_execution(&repository, lead).await;

    let unlinked = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "owner guard".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: None,
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("unlinked_participant")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap();
    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO conversation_execution_links \
             (id, conversation_id, execution_id, relation, active, created_at, updated_at) \
             VALUES ('foreign_owner_link', ?, ?, 'lead', 1, ?, ?)",
        )
        .bind(foreign_conversation)
        .bind(&unlinked.id)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "an Execution cannot link a Conversation owned by another account"
    );
    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO conversation_creation_keys \
             (creation_key, user_id, conversation_id, created_at) \
             VALUES ('foreign-key-owner', 'user_2', ?, ?)",
        )
        .bind(lead)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "a creation idempotency key cannot claim another owner's Conversation"
    );
    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO conversation_delivery_receipts \
             (operation_id, conversation_id, user_id, kind, request_payload, status, \
              created_at, updated_at) \
             VALUES ('foreign-receipt-owner', ?, 'user_2', 'turn', '{}', \
                     'accepted', ?, ?)",
        )
        .bind(lead)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "a delivery receipt cannot claim another owner's Conversation"
    );
    let removed_execution_tree_columns: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('agent_executions') \
         WHERE name IN ('parent_execution_id', 'delegation_depth')",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        removed_execution_tree_columns, 0,
        "Execution is one flat aggregate; recursion depth belongs only to Steps"
    );
    assert!(
        nomifun_db::sqlx::query("UPDATE conversations SET user_id = 'user_2' WHERE id = ?")
            .bind(lead)
            .execute(database.pool())
            .await
            .is_err(),
        "a linked Conversation owner is immutable"
    );

    repository
        .update_execution(
            USER_ID,
            &first_id,
            0,
            None,
            &UpdateAgentExecutionParams {
                status: Some(AgentExecutionStatus::Failed),
                ..Default::default()
            },
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    let replacement_id = create_execution(&repository, lead).await;
    assert_ne!(replacement_id, first_id);
    nomifun_db::sqlx::query("UPDATE agent_executions SET status = 'running' WHERE id = ?")
        .bind(&first_id)
        .execute(database.pool())
        .await
        .unwrap();
    let current_leads: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT execution_id FROM conversation_execution_links \
         WHERE conversation_id = ? AND relation = 'lead' AND active = 1",
    )
    .bind(lead)
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert_eq!(
        current_leads,
        vec![replacement_id],
        "historical execution state never creates a second current lead; only repository reopen commands may switch the immutable lead history"
    );
}

#[tokio::test]
async fn repository_and_schema_enforce_active_participant_limit() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead = create_conversation(&conversations, "participant limit lead").await;
    let execution_id = create_execution(&repository, lead).await;

    let mut excessive_participant_concurrency = participant("invalid_concurrency");
    excessive_participant_concurrency.constraints = Some(r#"{"max_concurrency":65}"#.to_owned());
    assert!(
        repository
            .create_execution_with_participants(
                USER_ID,
                &CreateAgentExecutionParams {
                    goal: "invalid participant concurrency".to_owned(),
                    status: AgentExecutionStatus::Planning,
                    plan_gate: PlanGate::Automatic,
                    adaptation_policy: AdaptationPolicy::Fixed,
                    decision_policy: DecisionPolicy::Automatic,
                    delegation_policy: DelegationPolicy::Automatic,
                    max_parallel: 1,
                    work_dir: None,
                    lead_conversation_id: Some(
                        create_conversation(&conversations, "invalid concurrency lead").await,
                    ),
                    initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
                },
                &[excessive_participant_concurrency],
                &event(AgentExecutionEventKind::Created),
            )
            .await
            .is_err()
    );
    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO agent_execution_participants ( \
                 id, execution_id, source_agent_id, provider_id, model, constraints, enabled_skills, \
                 disabled_builtin_skills, sort_order, introduced_in_revision, created_at \
             ) VALUES ('direct_invalid_concurrency', ?, 'agent_nomi', \
                       'provider_test', 'model_test', \
                       '{\"max_concurrency\":65}', '[]', '[]', 1, 0, ?)",
        )
        .bind(&execution_id)
        .bind(nomifun_common::now_ms())
        .execute(database.pool())
        .await
        .is_err(),
        "raw participant inserts share the 64 concurrency ceiling"
    );

    let too_many_for_create: Vec<_> =
        (0..=nomifun_common::MAX_AGENT_EXECUTION_PARTICIPANTS)
            .map(|index| participant(&format!("create_participant_{index}")))
            .collect();
    let create_error = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "oversized participant set".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: Some(
                    create_conversation(&conversations, "oversized create lead").await,
                ),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &too_many_for_create,
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap_err();
    assert!(
        create_error
            .to_string()
            .contains("64 active participants")
    );

    let mut too_many_for_reconcile = initial_plan(vec![agent_step(
        "participant_limit_step",
        "participant_1",
    )]);
    too_many_for_reconcile.new_participants =
        (0..nomifun_common::MAX_AGENT_EXECUTION_PARTICIPANTS)
            .map(|index| participant(&format!("replan_participant_{index}")))
            .collect();
    let reconcile_error = repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &too_many_for_reconcile,
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap_err();
    assert!(
        reconcile_error
            .to_string()
            .contains("64 active participants")
    );

    let now = nomifun_common::now_ms();
    nomifun_db::sqlx::query(
        "WITH RECURSIVE seq(i) AS ( \
             SELECT 2 UNION ALL SELECT i + 1 FROM seq WHERE i < 64 \
         ) \
         INSERT INTO agent_execution_participants ( \
             id, execution_id, source_agent_id, provider_id, model, enabled_skills, \
             disabled_builtin_skills, sort_order, introduced_in_revision, created_at \
         ) \
         SELECT 'direct_participant_' || i, ?, 'agent_nomi', \
                'provider_test', 'model_test', '[]', '[]', i, 0, ? \
         FROM seq",
    )
    .bind(&execution_id)
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();
    let active_count: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_execution_participants \
         WHERE execution_id = ? AND retired_in_revision IS NULL",
    )
    .bind(&execution_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        active_count,
        nomifun_common::MAX_AGENT_EXECUTION_PARTICIPANTS as i64
    );

    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO agent_execution_participants ( \
                 id, execution_id, source_agent_id, provider_id, model, enabled_skills, \
                 disabled_builtin_skills, sort_order, introduced_in_revision, created_at \
             ) VALUES ('participant_65', ?, 'agent_nomi', 'provider_test', 'model_test', \
                       '[]', '[]', 65, 0, ?)",
        )
        .bind(&execution_id)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "the schema must reject a 65th current participant even if a caller bypasses the repository",
    );

    nomifun_db::sqlx::query(
        "UPDATE agent_executions SET plan_revision = 1 WHERE id = ?",
    )
    .bind(&execution_id)
    .execute(database.pool())
    .await
    .unwrap();
    nomifun_db::sqlx::query(
        "UPDATE agent_execution_participants SET retired_in_revision = 1 \
         WHERE execution_id = ? AND id = 'direct_participant_64'",
    )
    .bind(&execution_id)
    .execute(database.pool())
    .await
    .unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO agent_execution_participants ( \
             id, execution_id, source_agent_id, provider_id, model, enabled_skills, \
             disabled_builtin_skills, sort_order, introduced_in_revision, created_at \
         ) VALUES ('replacement_participant', ?, 'agent_nomi', 'provider_test', 'model_test', \
                   '[]', '[]', 66, 1, ?)",
    )
    .bind(&execution_id)
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();
    let active_count: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_execution_participants \
         WHERE execution_id = ? AND retired_in_revision IS NULL",
    )
    .bind(&execution_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        active_count,
        nomifun_common::MAX_AGENT_EXECUTION_PARTICIPANTS as i64,
        "retired history does not consume the current participant budget"
    );
}

fn initial_plan(steps: Vec<NewAgentExecutionStep>) -> ReconcileAgentExecutionPlanParams {
    ReconcileAgentExecutionPlanParams {
        goal: None,
        plan_gate: None,
        adaptation_policy: None,
        decision_policy: None,
        delegation_policy: None,
        keep_step_ids: Vec::new(),
        new_participants: Vec::new(),
        retire_participant_ids: Vec::new(),
        new_steps: steps,
        new_dependencies: Vec::new(),
        execution_status: AgentExecutionStatus::Running,
    }
}

#[tokio::test]
async fn terminal_execution_reopen_atomically_restores_current_lead_for_every_control_path() {
    for reopen_path in ["append", "retry", "adopt"] {
        let database = test_database().await;
        let conversations = SqliteConversationRepository::new(database.pool().clone());
        let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
        let lead = create_conversation(&conversations, reopen_path).await;
        let first_id = create_execution(&repository, lead).await;

        let mut first_version = 0;
        let mut step_version = None;
        if reopen_path != "append" {
            repository
                .reconcile_plan(
                    USER_ID,
                    &first_id,
                    first_version,
                    &initial_plan(vec![agent_step("repair", "participant_1")]),
                    &event(AgentExecutionEventKind::PlanChanged),
                )
                .await
                .unwrap();
            first_version += 1;
            repository
                .transition_step_status(
                    USER_ID,
                    &first_id,
                    "repair",
                    first_version,
                    0,
                    None,
                    if reopen_path == "retry" {
                        ExecutionStepStatus::Completed
                    } else {
                        ExecutionStepStatus::Failed
                    },
                    &event(AgentExecutionEventKind::StepChanged),
                )
                .await
                .unwrap();
            first_version += 1;
            step_version = Some(1);
        }
        repository
            .update_execution(
                USER_ID,
                &first_id,
                first_version,
                None,
                &UpdateAgentExecutionParams {
                    status: Some(AgentExecutionStatus::Failed),
                    ..Default::default()
                },
                &event(AgentExecutionEventKind::StatusChanged),
            )
            .await
            .unwrap();
        first_version += 1;

        let second_id = create_execution(&repository, lead).await;
        repository
            .update_execution(
                USER_ID,
                &second_id,
                0,
                None,
                &UpdateAgentExecutionParams {
                    status: Some(AgentExecutionStatus::Failed),
                    ..Default::default()
                },
                &event(AgentExecutionEventKind::StatusChanged),
            )
            .await
            .unwrap();

        let first_before_reopen = repository
            .get_execution_detail(USER_ID, &first_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first_before_reopen.lead_conversation_id,
            Some(lead),
            "inactive lead history remains the Execution's immutable identity"
        );
        let current_before = repository
            .resolve_conversation_link(USER_ID, lead)
            .await
            .unwrap();
        assert_eq!(
            current_before
                .iter()
                .find(|link| link.relation == "lead" && link.active)
                .map(|link| link.execution_id.as_str()),
            Some(second_id.as_str())
        );

        match reopen_path {
            "append" => {
                repository
                    .append_steps(
                        USER_ID,
                        &first_id,
                        first_version,
                        &AppendAgentExecutionStepsParams {
                            new_steps: vec![agent_step("appended", "participant_1")],
                            new_dependencies: Vec::new(),
                        },
                        &event(AgentExecutionEventKind::PlanChanged),
                    )
                    .await
                    .unwrap();
            }
            "retry" => {
                repository
                    .reset_steps_for_retry(
                        USER_ID,
                        &first_id,
                        first_version,
                        &[RetryAgentExecutionStep {
                            step_id: "repair".to_owned(),
                            expected_step_version: step_version.unwrap(),
                        }],
                        &event(AgentExecutionEventKind::StepChanged),
                    )
                    .await
                    .unwrap();
            }
            "adopt" => {
                repository
                    .adopt_step_output(
                        USER_ID,
                        &first_id,
                        first_version,
                        "repair",
                        step_version.unwrap(),
                        &AdoptAgentExecutionStepOutputParams {
                            output_summary: "repaired output".to_owned(),
                            output_files: "[]".to_owned(),
                            tokens: None,
                            runtime_state: None,
                        },
                        &event(AgentExecutionEventKind::StepChanged),
                    )
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }

        let links = repository
            .resolve_conversation_link(USER_ID, lead)
            .await
            .unwrap();
        let active_leads: Vec<_> = links
            .iter()
            .filter(|link| link.relation == "lead" && link.active)
            .collect();
        assert_eq!(active_leads.len(), 1, "{reopen_path}");
        assert_eq!(active_leads[0].execution_id, first_id, "{reopen_path}");
        assert!(links.iter().any(|link| {
            link.relation == "lead" && !link.active && link.execution_id == second_id
        }));
        assert!(
            links
                .iter()
                .filter(|link| link.relation == "lead" && link.execution_id == first_id)
                .count()
                >= 2,
            "reopening appends a lead row without deleting historical identity: {reopen_path}"
        );
    }
}

#[tokio::test]
async fn running_attempt_appends_one_flat_dag_and_gates_only_pending_downstream() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead = create_conversation(&conversations, "append lead").await;
    let attempt_conversation = create_conversation(&conversations, "append caller").await;
    let execution_id = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "append in one aggregate".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: Some(lead),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("participant_1")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap()
        .id;
    let mut plan = initial_plan(vec![
        agent_step("caller", "participant_1"),
        agent_step("pending_after_caller", "participant_1"),
        agent_step("finished_after_caller", "participant_1"),
    ]);
    plan.new_dependencies = vec![
        NewAgentExecutionStepDependency {
            blocker_step_id: "caller".to_owned(),
            blocked_step_id: "pending_after_caller".to_owned(),
        },
        NewAgentExecutionStepDependency {
            blocker_step_id: "caller".to_owned(),
            blocked_step_id: "finished_after_caller".to_owned(),
        },
    ];
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &plan,
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE agent_execution_steps SET status = 'completed', version = version + 1 \
         WHERE execution_id = ? AND id = 'finished_after_caller'",
    )
    .bind(&execution_id)
    .execute(database.pool())
    .await
    .unwrap();

    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "caller",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
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
    let queued_attempt = queued.current_attempt.unwrap().attempt;
    let running = repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "caller",
            queued.step.version,
            &queued_attempt.id,
            queued_attempt.version,
            attempt_conversation,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let running_attempt = running.current_attempt.as_ref().unwrap().attempt.clone();
    let append = AppendAgentExecutionStepsFromAttemptParams {
        operation_id: "delegate-operation-1".to_owned(),
        caller_conversation_id: attempt_conversation,
        caller_step_id: "caller".to_owned(),
        caller_attempt_id: running_attempt.id.clone(),
        expected_caller_step_version: running.step.version,
        expected_caller_attempt_version: running_attempt.version,
        new_steps: vec![
            agent_step("delegated_root", "participant_1"),
            agent_step("delegated_leaf", "participant_1"),
        ],
        new_dependencies: vec![NewAgentExecutionStepDependency {
            blocker_step_id: "delegated_root".to_owned(),
            blocked_step_id: "delegated_leaf".to_owned(),
        }],
    };
    let append_event = NewAgentExecutionEvent {
        event_type: AgentExecutionEventKind::PlanChanged,
        step_id: None,
        attempt_id: None,
        actor: nomifun_common::AgentExecutionActor::agent(
            attempt_conversation,
            Some(running_attempt.id.clone()),
        ),
        payload: r#"{"change":"delegated_steps_appended"}"#.to_owned(),
    };
    let first_append = repository.append_steps_from_attempt(
        USER_ID,
        &execution_id,
        &append,
        &append_event,
    );
    let concurrent_replay = repository.append_steps_from_attempt(
        USER_ID,
        &execution_id,
        &append,
        &append_event,
    );
    let (appended, concurrently_replayed) = tokio::join!(first_append, concurrent_replay);
    let appended = appended.unwrap();
    let concurrently_replayed = concurrently_replayed.unwrap();
    assert_eq!(
        concurrently_replayed.added_step_ids,
        appended.added_step_ids,
        "SQLite's write fence makes a concurrent retry observe the first committed operation"
    );
    let detail = &appended.detail;

    assert_eq!(
        appended.added_step_ids,
        vec!["delegated_root".to_owned(), "delegated_leaf".to_owned()]
    );
    assert_eq!(detail.execution.max_parallel, 1, "a full caller slot does not reject append");
    assert_eq!(detail.execution.plan_revision, 2);
    for id in ["delegated_root", "delegated_leaf"] {
        let step = detail.steps.iter().find(|step| step.id == id).unwrap();
        assert_eq!(step.delegation_depth, 1);
        assert_eq!(step.status, "pending");
    }
    let caller = detail.steps.iter().find(|step| step.id == "caller").unwrap();
    let caller_attempt = detail
        .attempts
        .iter()
        .find(|attempt| attempt.attempt.id == running_attempt.id)
        .unwrap();
    assert_eq!(caller.version, running.step.version, "append preserves settlement fence");
    assert_eq!(
        caller_attempt.attempt.version, running_attempt.version,
        "append preserves the running Attempt generation"
    );
    let active_edges: std::collections::HashSet<_> = detail
        .dependencies
        .iter()
        .filter(|dependency| dependency.superseded_in_revision.is_none())
        .map(|dependency| {
            (
                dependency.blocker_step_id.as_str(),
                dependency.blocked_step_id.as_str(),
            )
        })
        .collect();
    assert!(active_edges.contains(&("delegated_root", "delegated_leaf")));
    assert!(active_edges.contains(&("delegated_leaf", "pending_after_caller")));
    assert!(!active_edges.contains(&("delegated_leaf", "finished_after_caller")));
    assert!(!active_edges.contains(&("caller", "delegated_root")));

    let now = nomifun_common::now_ms();
    sqlx::query(
        "UPDATE agent_execution_attempts SET status = 'completed', finished_at = ?, \
            version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND step_id = 'caller' AND id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(&execution_id)
    .bind(&running_attempt.id)
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
         WHERE execution_id = ? AND attempt_id = ?",
    )
    .bind(now)
    .bind(&execution_id)
    .bind(&running_attempt.id)
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE agent_execution_steps SET status = 'completed', version = version + 1, \
            updated_at = ? WHERE execution_id = ? AND id = 'caller'",
    )
    .bind(now)
    .bind(&execution_id)
    .execute(database.pool())
    .await
    .unwrap();

    let replay = repository
        .append_steps_from_attempt(
            USER_ID,
            &execution_id,
            &AppendAgentExecutionStepsFromAttemptParams {
                operation_id: append.operation_id.clone(),
                caller_conversation_id: attempt_conversation,
                caller_step_id: "caller".to_owned(),
                caller_attempt_id: running_attempt.id.clone(),
                expected_caller_step_version: -1,
                expected_caller_attempt_version: -1,
                new_steps: vec![agent_step("fresh_replay_candidate", "participant_1")],
                new_dependencies: Vec::new(),
            },
            &NewAgentExecutionEvent {
                event_type: AgentExecutionEventKind::PlanChanged,
                step_id: None,
                attempt_id: None,
                actor: nomifun_common::AgentExecutionActor::agent(
                    attempt_conversation,
                    Some(running_attempt.id),
                ),
                payload: "{}".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(replay.added_step_ids, appended.added_step_ids);
    assert_eq!(replay.detail.execution.plan_revision, 2);
    assert!(
        replay
            .detail
            .steps
            .iter()
            .all(|step| step.id != "fresh_replay_candidate"),
        "a lost-response replay returns the original batch instead of materializing new ids"
    );
    let operation_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_execution_events \
         WHERE execution_id = ? AND json_extract(payload, '$.operation_id') = ?",
    )
    .bind(&execution_id)
    .bind(&append.operation_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(operation_events, 1);
    let probed = repository
        .find_steps_append_from_attempt(USER_ID, &execution_id, &append.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(probed.added_step_ids, appended.added_step_ids);
    assert!(
        repository
            .find_steps_append_from_attempt(USER_ID, &execution_id, "unknown-operation")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn versioned_append_preserves_history_and_reopens_a_settled_execution() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "settled append lead").await,
    )
    .await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("historical_step", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE agent_execution_steps SET status = 'completed', version = version + 1 \
         WHERE execution_id = ? AND id = 'historical_step'",
    )
    .bind(&execution_id)
    .execute(database.pool())
    .await
    .unwrap();
    let settled = repository
        .update_execution(
            USER_ID,
            &execution_id,
            1,
            None,
            &UpdateAgentExecutionParams {
                status: Some(AgentExecutionStatus::Completed),
                ..Default::default()
            },
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    let detail = repository
        .append_steps(
            USER_ID,
            &execution_id,
            settled.version,
            &AppendAgentExecutionStepsParams {
                new_steps: vec![agent_step("appended_after_settlement", "participant_1")],
                new_dependencies: Vec::new(),
            },
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    assert_eq!(detail.execution.status, "running");
    assert_eq!(detail.execution.plan_revision, 2);
    let historical = detail
        .steps
        .iter()
        .find(|step| step.id == "historical_step")
        .unwrap();
    assert_eq!(historical.status, "completed");
    assert_eq!(historical.introduced_in_revision, 1);
    assert!(historical.superseded_in_revision.is_none());
    let appended = detail
        .steps
        .iter()
        .find(|step| step.id == "appended_after_settlement")
        .unwrap();
    assert_eq!(appended.status, "pending");
    assert_eq!(appended.delegation_depth, 0);
    assert_eq!(appended.introduced_in_revision, 2);

    assert!(
        repository
            .append_steps(
                USER_ID,
                &execution_id,
                settled.version,
                &AppendAgentExecutionStepsParams {
                    new_steps: vec![agent_step("stale_append", "participant_1")],
                    new_dependencies: Vec::new(),
                },
                &event(AgentExecutionEventKind::PlanChanged),
            )
            .await
            .is_err(),
        "the aggregate version remains the user/lead append fence"
    );
}

#[tokio::test]
async fn pause_atomically_interrupts_in_flight_work_and_fences_the_old_generation() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let attempt_conversation = create_conversation(&conversations, "in-flight attempt").await;
    let lead_conversation_id = create_conversation(&conversations, "lead").await;
    let execution_id = create_execution(&repository, lead_conversation_id).await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![
                agent_step("step_running", "participant_1"),
                agent_step("step_queued", "participant_1"),
            ]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();

    let first = AgentExecutionLeaseToken::new("generation-1".to_owned());
    let first_expiry = nomifun_common::now_ms() + 60_000;
    repository
        .try_acquire_lease(&execution_id, 1, first.owner(), first_expiry)
        .await
        .unwrap()
        .unwrap();

    let running_attempt = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "step_running",
            0,
            Some(&first),
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
                start_immediately: false,
                trigger_reason: "initial".to_owned(),
                effective_config: "{}".to_owned(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap()
        .current_attempt
        .unwrap()
        .attempt;
    repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "step_running",
            1,
            &running_attempt.id,
            0,
            attempt_conversation,
            Some(&first),
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "step_queued",
            0,
            Some(&first),
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
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

    let before_pause = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    let paused = repository
        .pause_execution(
            USER_ID,
            &execution_id,
            before_pause.version,
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    assert_eq!(paused.status, "paused");
    assert_eq!((paused.lease_owner, paused.lease_expires_at), (None, None));

    let detail = repository
        .get_execution_detail(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    let statuses: std::collections::HashMap<_, _> = detail
        .attempts
        .iter()
        .map(|attempt| (attempt.attempt.step_id.as_str(), attempt.attempt.status.as_str()))
        .collect();
    assert_eq!(statuses.get("step_running"), Some(&"interrupted"));
    assert_eq!(statuses.get("step_queued"), Some(&"cancelled"));
    assert!(
        detail
            .steps
            .iter()
            .all(|step| step.status == "pending"),
        "running work returns to Pending and queued work remains Pending"
    );
    let attempt_links = repository
        .list_conversation_links(USER_ID, &execution_id)
        .await
        .unwrap();
    assert!(attempt_links.iter().any(|link| {
        link.relation == "attempt"
            && link.attempt_id.as_deref() == Some(running_attempt.id.as_str())
            && !link.active
            && link.cleanup_completed_at.is_none()
    }));
    assert_eq!(
        repository
            .list_pending_conversation_cleanups(Some(&execution_id), 10)
            .await
            .unwrap()
            .len(),
        1
    );

    assert!(
        repository
            .try_acquire_lease(
                &execution_id,
                paused.version,
                "generation-while-paused",
                nomifun_common::now_ms() + 60_000,
            )
            .await
            .unwrap()
            .is_none(),
        "Paused cannot acquire a scheduler lease"
    );
    assert!(
        repository
            .create_attempt(
                USER_ID,
                &execution_id,
                "step_queued",
                1,
                Some(&first),
                &CreateAgentExecutionAttemptParams {
                    participant_id: Some("participant_1".to_owned()),
                    start_immediately: false,
                    trigger_reason: "stale".to_owned(),
                    effective_config: "{}".to_owned(),
                    retry_after: None,
                    runtime_state: None,
                },
                &event(AgentExecutionEventKind::AttemptChanged),
            )
            .await
            .is_err(),
        "the revoked generation cannot persist a callback after Pause"
    );

    let resumed = repository
        .resume_execution(
            USER_ID,
            &execution_id,
            paused.version,
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, "running");
    let second = AgentExecutionLeaseToken::new("generation-2".to_owned());
    repository
        .try_acquire_lease(
            &execution_id,
            resumed.version,
            second.owner(),
            nomifun_common::now_ms() + 60_000,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        repository
            .create_attempt(
                USER_ID,
                &execution_id,
                "step_queued",
                1,
                Some(&first),
                &CreateAgentExecutionAttemptParams {
                    participant_id: Some("participant_1".to_owned()),
                    start_immediately: false,
                    trigger_reason: "stale".to_owned(),
                    effective_config: "{}".to_owned(),
                    retry_after: None,
                    runtime_state: None,
                },
                &event(AgentExecutionEventKind::AttemptChanged),
            )
            .await
            .is_err(),
        "a superseded scheduler generation must not write after resume"
    );
    repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "step_queued",
            1,
            Some(&second),
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
                start_immediately: false,
                trigger_reason: "current".to_owned(),
                effective_config: "{}".to_owned(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn pause_preserves_waiting_questions_and_resume_restores_waiting_input() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let attempt_conversation = create_conversation(&conversations, "waiting attempt").await;
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "waiting lead").await,
    )
    .await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("waiting_step", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "waiting_step",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
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
    let attempt = queued.current_attempt.as_ref().unwrap().attempt.clone();
    let running = repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "waiting_step",
            queued.step.version,
            &attempt.id,
            attempt.version,
            attempt_conversation,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let running_attempt = running.current_attempt.as_ref().unwrap().attempt.clone();
    repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "waiting_step",
            running.step.version,
            &attempt.id,
            running_attempt.version,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::WaitingInput,
                step_status: ExecutionStepStatus::WaitingInput,
                execution_status: Some(AgentExecutionStatus::WaitingInput),
                question: Some(Some("keep this question?".to_owned())),
                error: None,
                output_summary: None,
                output_files: None,
                tokens: None,
                retry_after: None,
                runtime_state: None,
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::DecisionRequested),
        )
        .await
        .unwrap();
    let before_pause = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    let paused = repository
        .pause_execution(
            USER_ID,
            &execution_id,
            before_pause.version,
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    assert_eq!(paused.status, "paused");

    let waiting = repository
        .get_attempt(USER_ID, &execution_id, "waiting_step", &attempt.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.attempt.status, "waiting_input");
    assert_eq!(waiting.attempt.question.as_deref(), Some("keep this question?"));
    assert_eq!(waiting.conversation_id, Some(attempt_conversation));
    assert!(
        repository
            .list_conversation_links(USER_ID, &execution_id)
            .await
            .unwrap()
            .iter()
            .any(|link| link.attempt_id.as_deref() == Some(attempt.id.as_str()) && link.active),
        "WaitingInput keeps its active decision route while paused"
    );

    let resumed = repository
        .resume_execution(
            USER_ID,
            &execution_id,
            paused.version,
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, "waiting_input");
}

#[tokio::test]
async fn attempt_lifecycle_resume_and_adopt_are_atomic() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead_conversation_id = create_conversation(&conversations, "lead").await;
    let attempt_conversation_id = create_conversation(&conversations, "attempt").await;
    let execution_id = create_execution(&repository, lead_conversation_id).await;

    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("step_1", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "step_1",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_string()),
                start_immediately: false,
                trigger_reason: "initial".to_string(),
                effective_config: "{}".to_string(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_id = queued.current_attempt.unwrap().attempt.id;

    let active_mutation = repository
        .transition_step_status(
            USER_ID,
            &execution_id,
            "step_1",
            2,
            1,
            None,
            ExecutionStepStatus::Skipped,
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await;
    assert!(active_mutation.is_err());

    let running = repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "step_1",
            1,
            &attempt_id,
            0,
            attempt_conversation_id,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    assert_eq!(running.step.status, "running");
    assert_eq!(running.current_attempt.as_ref().unwrap().attempt.version, 1);
    assert!(
        repository
            .has_attempt_conversation_link(USER_ID, attempt_conversation_id)
            .await
            .unwrap()
    );
    assert!(
        !repository
            .has_attempt_conversation_link("another_user", attempt_conversation_id)
            .await
            .unwrap()
    );
    assert!(
        !repository
            .has_attempt_conversation_link(USER_ID, lead_conversation_id)
            .await
            .unwrap()
    );
    assert_eq!(
        repository
            .get_execution(USER_ID, &execution_id)
            .await
            .unwrap()
            .unwrap()
            .version,
        3
    );

    let waiting = repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "step_1",
            2,
            &attempt_id,
            1,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::WaitingInput,
                step_status: ExecutionStepStatus::WaitingInput,
                execution_status: Some(AgentExecutionStatus::WaitingInput),
                question: Some(Some("approve?".to_string())),
                error: None,
                output_summary: None,
                output_files: None,
                tokens: None,
                retry_after: None,
                runtime_state: None,
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    assert_eq!(waiting.current_attempt.as_ref().unwrap().attempt.question.as_deref(), Some("approve?"));

    let resumed = repository
        .resume_waiting_attempt(
            USER_ID,
            &execution_id,
            4,
            "step_1",
            3,
            &attempt_id,
            2,
            &AttemptConversationEffectParams {
                runtime_state: Some(
                    r#"{"pending_conversation_effects":[{"kind":"decision_input","operation_id":"effect_1","content":"yes"}]}"#
                        .to_owned(),
                ),
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    assert_eq!(resumed.conversation_id, attempt_conversation_id);
    assert_eq!(resumed.detail.step.status, "running");
    assert!(
        resumed
            .detail
            .current_attempt
            .as_ref()
            .unwrap()
            .attempt
            .question
            .is_none()
    );

    let completed = repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "step_1",
            4,
            &attempt_id,
            3,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::Completed,
                step_status: ExecutionStepStatus::Completed,
                execution_status: Some(AgentExecutionStatus::Completed),
                question: None,
                error: None,
                output_summary: Some(Some("original output".to_string())),
                output_files: Some("[]".to_string()),
                tokens: Some(Some(10)),
                retry_after: None,
                runtime_state: None,
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    assert_eq!(completed.current_attempt.as_ref().unwrap().attempt.status, "completed");
    assert!(
        repository
            .has_attempt_conversation_link(USER_ID, attempt_conversation_id)
            .await
            .unwrap(),
        "inactive attempt links remain part of the execution audit trail"
    );
    let settled_attempt_links = repository
        .list_conversation_links(USER_ID, &execution_id)
        .await
        .unwrap();
    assert!(settled_attempt_links.iter().any(|link| {
        link.conversation_id == attempt_conversation_id
            && link.relation == "attempt"
            && !link.active
    }));

    let execution_count_before_reuse: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_executions WHERE user_id = ?",
    )
    .bind(USER_ID)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let create_from_historical_attempt = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "illegal historical Attempt reuse".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: Some(attempt_conversation_id),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("illegal_reuse_participant")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap_err();
    assert!(
        create_from_historical_attempt
            .to_string()
            .contains("permanently belongs")
    );
    let execution_count_after_reuse: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_executions WHERE user_id = ?",
    )
    .bind(USER_ID)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        execution_count_after_reuse, execution_count_before_reuse,
        "a rejected create cannot leave an unlinked Execution behind"
    );

    let schema_guard_target = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "schema guard target".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: None,
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("schema_guard_participant")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "INSERT INTO conversation_execution_links \
             (id, conversation_id, execution_id, relation, active, created_at, updated_at) \
             VALUES ('historical_attempt_illegal_lead', ?, ?, 'lead', 1, 1, 1)",
        )
        .bind(attempt_conversation_id)
        .bind(&schema_guard_target.id)
        .execute(database.pool())
        .await
        .is_err(),
        "the schema boundary retains Attempt identity even after its link becomes inactive"
    );
    let target_link_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_execution_links WHERE execution_id = ?",
    )
    .bind(&schema_guard_target.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(target_link_count, 0);

    let adopted = repository
        .adopt_step_output(
            USER_ID,
            &execution_id,
            6,
            "step_1",
            5,
            &AdoptAgentExecutionStepOutputParams {
                output_summary: "new conversation output".to_string(),
                output_files: "[]".to_string(),
                tokens: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    let adopted_attempt = adopted.current_attempt.unwrap();
    assert_eq!(adopted_attempt.attempt.attempt_no, 1);
    assert_eq!(adopted_attempt.attempt.trigger_reason, "adopt");
    assert_eq!(adopted_attempt.attempt.output_summary.as_deref(), Some("new conversation output"));
    assert_eq!(adopted_attempt.conversation_id, Some(attempt_conversation_id));

    let execution = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(execution.status, "running");
    assert_eq!(execution.version, 7);
    assert_eq!(execution.event_sequence, 8);
    assert_eq!(
        repository
            .list_attempts(USER_ID, &execution_id, Some("step_1"))
            .await
            .unwrap()
            .len(),
        2
    );

    assert!(
        repository
            .delete_execution(USER_ID, &execution_id, 7, &event(AgentExecutionEventKind::Deleted))
            .await
            .unwrap()
    );
    assert!(
        repository
            .has_attempt_conversation_link(USER_ID, attempt_conversation_id)
            .await
            .unwrap(),
        "soft deletion must not make an attempt transcript physically deletable"
    );
}

#[tokio::test]
async fn waiting_attempt_can_acknowledge_a_delivered_stop_turn_effect() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead_conversation_id = create_conversation(&conversations, "stop effect lead").await;
    let attempt_conversation_id =
        create_conversation(&conversations, "stop effect attempt").await;
    let execution_id = create_execution(&repository, lead_conversation_id).await;

    let execution = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            execution.version,
            &initial_plan(vec![agent_step("stop_step", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let step = repository
        .get_step(USER_ID, &execution_id, "stop_step")
        .await
        .unwrap()
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "stop_step",
            step.version,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
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
    let queued_attempt = queued.current_attempt.unwrap().attempt;
    let running = repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "stop_step",
            queued.step.version,
            &queued_attempt.id,
            queued_attempt.version,
            attempt_conversation_id,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let running_attempt = running.current_attempt.as_ref().unwrap().attempt.clone();
    let stop_effect = r#"{"pending_conversation_effects":[{"kind":"stop_turn","operation_id":"stop:1"}]}"#;
    let waiting = repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "stop_step",
            running.step.version,
            &running_attempt.id,
            running_attempt.version,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::WaitingInput,
                step_status: ExecutionStepStatus::WaitingInput,
                execution_status: Some(AgentExecutionStatus::WaitingInput),
                question: Some(Some("need user input".to_owned())),
                error: None,
                output_summary: None,
                output_files: None,
                tokens: None,
                retry_after: None,
                runtime_state: Some(Some(stop_effect.to_owned())),
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::DecisionRequested),
        )
        .await
        .unwrap();
    let waiting_attempt = waiting.current_attempt.as_ref().unwrap().attempt.clone();
    assert_eq!(waiting_attempt.status, "waiting_input");
    assert_eq!(waiting_attempt.runtime_state.as_deref(), Some(stop_effect));

    let acknowledged = repository
        .acknowledge_attempt_conversation_effect(
            USER_ID,
            &execution_id,
            "stop_step",
            &waiting_attempt.id,
            waiting_attempt.version,
            &AttemptConversationEffectParams {
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let acknowledged_attempt = acknowledged.current_attempt.unwrap().attempt;
    assert_eq!(acknowledged_attempt.status, "waiting_input");
    assert_eq!(acknowledged_attempt.question.as_deref(), Some("need user input"));
    assert!(acknowledged_attempt.runtime_state.is_none());
}

#[tokio::test]
async fn resuming_one_of_multiple_waiting_attempts_keeps_the_aggregate_waiting() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "multi-wait lead").await,
    )
    .await;
    let conversation_a = create_conversation(&conversations, "multi-wait A").await;
    let conversation_b = create_conversation(&conversations, "multi-wait B").await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![
                agent_step("wait_a", "participant_1"),
                agent_step("wait_b", "participant_1"),
            ]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();

    let queued_a = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "wait_a",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
                start_immediately: false,
                trigger_reason: "parallel".to_owned(),
                effective_config: "{}".to_owned(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_a = queued_a.current_attempt.unwrap().attempt;
    let queued_b = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "wait_b",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_owned()),
                start_immediately: false,
                trigger_reason: "parallel".to_owned(),
                effective_config: "{}".to_owned(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_b = queued_b.current_attempt.unwrap().attempt;

    let running_a = repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "wait_a",
            queued_a.step.version,
            &attempt_a.id,
            attempt_a.version,
            conversation_a,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let running_attempt_a = running_a.current_attempt.as_ref().unwrap().attempt.clone();
    let running_b = repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "wait_b",
            queued_b.step.version,
            &attempt_b.id,
            attempt_b.version,
            conversation_b,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let running_attempt_b = running_b.current_attempt.as_ref().unwrap().attempt.clone();

    let wait_params = |question: &str| SettleAgentExecutionAttemptParams {
        attempt_status: ExecutionAttemptStatus::WaitingInput,
        step_status: ExecutionStepStatus::WaitingInput,
        execution_status: Some(AgentExecutionStatus::WaitingInput),
        question: Some(Some(question.to_owned())),
        error: None,
        output_summary: None,
        output_files: None,
        tokens: None,
        retry_after: None,
        runtime_state: None,
        started_at: None,
        finished_at: None,
        loop_repeat_reset: None,
    };
    let waiting_a = repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "wait_a",
            running_a.step.version,
            &attempt_a.id,
            running_attempt_a.version,
            None,
            &wait_params("answer A"),
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let waiting_b = repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "wait_b",
            running_b.step.version,
            &attempt_b.id,
            running_attempt_b.version,
            None,
            &wait_params("answer B"),
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();

    let execution = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(execution.status, "waiting_input");
    let waiting_attempt_a = waiting_a.current_attempt.as_ref().unwrap().attempt.clone();
    let resumed_a = repository
        .resume_waiting_attempt(
            USER_ID,
            &execution_id,
            execution.version,
            "wait_a",
            waiting_a.step.version,
            &attempt_a.id,
            waiting_attempt_a.version,
            &AttemptConversationEffectParams {
                runtime_state: Some(
                    r#"{"pending_conversation_effects":[{"kind":"decision_input","operation_id":"answer:a","content":"A"}]}"#
                        .to_owned(),
                ),
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    assert_eq!(resumed_a.conversation_id, conversation_a);
    assert_eq!(resumed_a.detail.step.status, "running");
    let after_a = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_a.status, "waiting_input",
        "another WaitingInput attempt keeps the aggregate in WaitingInput"
    );
    assert_eq!(
        repository
            .get_attempt(USER_ID, &execution_id, "wait_b", &attempt_b.id)
            .await
            .unwrap()
            .unwrap()
            .attempt
            .status,
        "waiting_input"
    );

    let waiting_attempt_b = waiting_b.current_attempt.as_ref().unwrap().attempt.clone();
    let resumed_b = repository
        .resume_waiting_attempt(
            USER_ID,
            &execution_id,
            after_a.version,
            "wait_b",
            waiting_b.step.version,
            &attempt_b.id,
            waiting_attempt_b.version,
            &AttemptConversationEffectParams {
                runtime_state: Some(
                    r#"{"pending_conversation_effects":[{"kind":"decision_input","operation_id":"answer:b","content":"B"}]}"#
                        .to_owned(),
                ),
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    assert_eq!(resumed_b.conversation_id, conversation_b);
    let after_b = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_b.status, "running");

    let cancelled = repository
        .cancel_execution(
            USER_ID,
            &execution_id,
            after_b.version,
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    assert!(
        cancelled
            .attempts
            .iter()
            .all(|attempt| attempt.attempt.status == "cancelled"),
        "user cancellation is distinct from process interruption"
    );
    let pending = repository
        .list_pending_conversation_cleanups(Some(&execution_id), 10)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        repository
            .mark_conversation_cleanup_completed(&pending[0].link_id, nomifun_common::now_ms())
            .await
            .unwrap()
    );
    assert_eq!(
        repository
            .list_pending_conversation_cleanups(Some(&execution_id), 10)
            .await
            .unwrap()
            .len(),
        1,
        "cleanup intent remains durable until each external cancellation is acknowledged"
    );
}

#[tokio::test]
async fn replan_interrupts_superseded_attempt_and_preserves_revision_history() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead_conversation_id = create_conversation(&conversations, "lead").await;
    let execution_id = create_execution(&repository, lead_conversation_id).await;
    let mut plan = initial_plan(vec![
        agent_step("step_keep", "participant_1"),
        agent_step("step_drop", "participant_1"),
    ]);
    plan.new_dependencies = vec![NewAgentExecutionStepDependency {
        blocker_step_id: "step_keep".to_string(),
        blocked_step_id: "step_drop".to_string(),
    }];
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &plan,
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "step_drop",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_string()),
                start_immediately: false,
                trigger_reason: "initial".to_string(),
                effective_config: "{}".to_string(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_id = queued.current_attempt.unwrap().attempt.id;

    let detail = repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            2,
            &ReconcileAgentExecutionPlanParams {
                goal: Some("replanned goal".to_string()),
                plan_gate: Some(PlanGate::RequireApproval),
                adaptation_policy: Some(AdaptationPolicy::Adaptive),
                decision_policy: Some(DecisionPolicy::AskUser),
                delegation_policy: Some(DelegationPolicy::PreferParallel),
                keep_step_ids: vec!["step_keep".to_string()],
                new_participants: Vec::new(),
                retire_participant_ids: Vec::new(),
                new_steps: vec![agent_step("step_new", "participant_1")],
                new_dependencies: vec![NewAgentExecutionStepDependency {
                    blocker_step_id: "step_keep".to_string(),
                    blocked_step_id: "step_new".to_string(),
                }],
                execution_status: AgentExecutionStatus::AwaitingApproval,
            },
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();

    assert_eq!(detail.execution.goal, "replanned goal");
    assert_eq!(detail.execution.plan_gate, "require_approval");
    assert_eq!(detail.execution.adaptation_policy, "adaptive");
    assert_eq!(detail.execution.decision_policy, "ask_user");
    assert_eq!(detail.execution.delegation_policy, "prefer_parallel");
    assert_eq!(detail.execution.plan_revision, 2);
    assert_eq!(detail.steps.len(), 3);
    assert_eq!(detail.dependencies.len(), 2);
    let superseded = detail.steps.iter().find(|step| step.id == "step_drop").unwrap();
    assert_eq!(superseded.status, "skipped");
    assert_eq!(superseded.superseded_in_revision, Some(2));
    assert_eq!(
        detail
            .attempts
            .iter()
            .find(|attempt| attempt.attempt.id == attempt_id)
            .unwrap()
            .attempt
            .status,
        "cancelled"
    );
    assert!(
        repository
            .get_step(USER_ID, &execution_id, "step_drop")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repository
            .list_steps(USER_ID, &execution_id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn parallel_attempt_creation_advances_aggregate_version_without_false_conflict() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![
                agent_step("step_parallel_1", "participant_1"),
                agent_step("step_parallel_2", "participant_1"),
            ]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();

    let params_1 = CreateAgentExecutionAttemptParams {
        participant_id: Some("participant_1".to_string()),
        start_immediately: false,
        trigger_reason: "parallel".to_string(),
        effective_config: "{}".to_string(),
        retry_after: None,
        runtime_state: None,
    };
    let params_2 = params_1.clone();
    let event_1 = event(AgentExecutionEventKind::AttemptChanged);
    let event_2 = event(AgentExecutionEventKind::AttemptChanged);
    let (attempt_1, attempt_2) = tokio::join!(
        repository.create_attempt(
            USER_ID,
            &execution_id,
            "step_parallel_1",
            0,
            None,
            &params_1,
            &event_1,
        ),
        repository.create_attempt(
            USER_ID,
            &execution_id,
            "step_parallel_2",
            0,
            None,
            &params_2,
            &event_2,
        )
    );
    assert_eq!(
        attempt_1
            .unwrap()
            .current_attempt
            .unwrap()
            .attempt
            .status,
        "queued"
    );
    assert_eq!(
        attempt_2
            .unwrap()
            .current_attempt
            .unwrap()
            .attempt
            .status,
        "queued"
    );
    let execution = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(execution.version, 3);
    assert_eq!(execution.event_sequence, 4);
}

#[tokio::test]
async fn explicit_retry_reopens_a_settled_execution_but_never_a_cancelled_one() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    let attempt_conversation_id = create_conversation(&conversations, "attempt").await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("step_retry", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "step_retry",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_string()),
                start_immediately: false,
                trigger_reason: "initial".to_string(),
                effective_config: "{}".to_string(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_id = queued.current_attempt.unwrap().attempt.id;
    repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "step_retry",
            1,
            &attempt_id,
            0,
            attempt_conversation_id,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "step_retry",
            2,
            &attempt_id,
            1,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::Completed,
                step_status: ExecutionStepStatus::Completed,
                execution_status: Some(AgentExecutionStatus::Completed),
                question: None,
                error: None,
                output_summary: Some(Some("done".to_string())),
                output_files: Some("[]".to_string()),
                tokens: None,
                retry_after: None,
                runtime_state: None,
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();

    assert!(
        repository
            .update_execution(
                USER_ID,
                &execution_id,
                4,
                None,
                &UpdateAgentExecutionParams {
                    status: Some(AgentExecutionStatus::Running),
                    ..Default::default()
                },
                &event(AgentExecutionEventKind::StatusChanged),
            )
            .await
            .is_err(),
        "settled executions may reopen only through retry/adopt"
    );
    let reopened = repository
        .reset_steps_for_retry(
            USER_ID,
            &execution_id,
            4,
            &[RetryAgentExecutionStep {
                step_id: "step_retry".to_string(),
                expected_step_version: 3,
            }],
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    assert_eq!(reopened.execution.status, "running");
    assert_eq!(reopened.execution.version, 5);
    assert_eq!(reopened.steps[0].status, "pending");

    let cancelled = repository
        .cancel_execution(
            USER_ID,
            &execution_id,
            5,
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.execution.status, "cancelled");
    assert!(
        repository
            .reset_steps_for_retry(
                USER_ID,
                &execution_id,
                6,
                &[RetryAgentExecutionStep {
                    step_id: "step_retry".to_string(),
                    expected_step_version: 5,
                }],
                &event(AgentExecutionEventKind::StepChanged),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_execution_cannot_retry_or_adopt_a_completed_step() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    let attempt_conversation_id = create_conversation(&conversations, "attempt").await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("completed_before_cancel", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "completed_before_cancel",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_string()),
                start_immediately: false,
                trigger_reason: "initial".to_string(),
                effective_config: "{}".to_string(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_id = queued.current_attempt.unwrap().attempt.id;
    repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "completed_before_cancel",
            1,
            &attempt_id,
            0,
            attempt_conversation_id,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "completed_before_cancel",
            2,
            &attempt_id,
            1,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::Completed,
                step_status: ExecutionStepStatus::Completed,
                execution_status: None,
                question: None,
                error: None,
                output_summary: Some(Some("done".to_string())),
                output_files: Some("[]".to_string()),
                tokens: None,
                retry_after: None,
                runtime_state: None,
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let cancelled = repository
        .cancel_execution(USER_ID, &execution_id, 4, &event(AgentExecutionEventKind::StatusChanged))
        .await
        .unwrap();
    assert_eq!(cancelled.execution.status, "cancelled");
    assert_eq!(cancelled.steps[0].status, "completed");

    assert!(
        repository
            .reset_steps_for_retry(
                USER_ID,
                &execution_id,
                5,
                &[RetryAgentExecutionStep {
                    step_id: "completed_before_cancel".to_string(),
                    expected_step_version: 3,
                }],
                &event(AgentExecutionEventKind::StepChanged),
            )
            .await
            .is_err()
    );
    assert!(
        repository
            .adopt_step_output(
                USER_ID,
                &execution_id,
                5,
                "completed_before_cancel",
                3,
                &AdoptAgentExecutionStepOutputParams {
                    output_summary: "late output".to_string(),
                    output_files: "[]".to_string(),
                    tokens: None,
                    runtime_state: None,
                },
                &event(AgentExecutionEventKind::StepChanged),
            )
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE agent_executions SET status = 'running' WHERE id = ?")
            .bind(&execution_id)
            .execute(database.pool())
            .await
            .is_err(),
        "the DB transition guard must make Cancelled irreversible"
    );
    let unchanged = repository
        .get_execution_detail(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.execution.status, "cancelled");
    assert_eq!(unchanged.execution.version, 5);
    assert_eq!(unchanged.steps[0].status, "completed");
    assert_eq!(unchanged.attempts.len(), 1);
}

#[tokio::test]
async fn loop_repeat_settlement_and_body_closure_reset_are_one_cas_transaction() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    let mut plan = initial_plan(vec![
        agent_step("body", "participant_1"),
        loop_step("controller"),
        agent_step("downstream", "participant_1"),
    ]);
    plan.new_dependencies = vec![
        NewAgentExecutionStepDependency {
            blocker_step_id: "body".to_string(),
            blocked_step_id: "controller".to_string(),
        },
        NewAgentExecutionStepDependency {
            blocker_step_id: "controller".to_string(),
            blocked_step_id: "downstream".to_string(),
        },
    ];
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &plan,
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    repository
        .transition_step_status(
            USER_ID,
            &execution_id,
            "body",
            1,
            0,
            None,
            ExecutionStepStatus::Completed,
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    repository
        .transition_step_status(
            USER_ID,
            &execution_id,
            "downstream",
            2,
            0,
            None,
            ExecutionStepStatus::Completed,
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    let running = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "controller",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: None,
                start_immediately: true,
                trigger_reason: "control_evaluation".to_string(),
                effective_config: r#"{"kind":"loop"}"#.to_string(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt = running.current_attempt.unwrap().attempt;
    let settle = |downstream_version| SettleAgentExecutionAttemptParams {
        attempt_status: ExecutionAttemptStatus::Completed,
        step_status: ExecutionStepStatus::Pending,
        execution_status: None,
        question: Some(None),
        error: Some(None),
        output_summary: Some(Some("repeat".to_string())),
        output_files: Some("[]".to_string()),
        tokens: Some(None),
        retry_after: Some(None),
        runtime_state: Some(Some(r#"{"iteration":1}"#.to_string())),
        started_at: None,
        finished_at: None,
        loop_repeat_reset: Some(LoopRepeatResetParams {
            body_step_id: "body".to_string(),
            expected_steps: vec![
                RetryAgentExecutionStep {
                    step_id: "body".to_string(),
                    expected_step_version: 1,
                },
                RetryAgentExecutionStep {
                    step_id: "downstream".to_string(),
                    expected_step_version: downstream_version,
                },
            ],
        }),
    };

    assert!(
        repository
            .settle_attempt(
                USER_ID,
                &execution_id,
                "controller",
                1,
                &attempt.id,
                0,
                None,
                &settle(0),
                &event(AgentExecutionEventKind::StepChanged),
            )
            .await
            .is_err(),
        "one stale descendant must roll back the entire Repeat transition"
    );
    let rolled_back = repository
        .get_execution_detail(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rolled_back.execution.version, 4);
    assert_eq!(
        rolled_back
            .steps
            .iter()
            .find(|step| step.id == "controller")
            .unwrap()
            .status,
        "running"
    );
    assert_eq!(
        rolled_back
            .attempts
            .iter()
            .find(|candidate| candidate.attempt.id == attempt.id)
            .unwrap()
            .attempt
            .status,
        "running"
    );
    assert!(rolled_back.steps.iter().filter(|step| step.id != "controller").all(|step| step.status == "completed"));

    let committed = repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "controller",
            1,
            &attempt.id,
            0,
            None,
            &settle(1),
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    assert_eq!(committed.step.status, "pending");
    assert_eq!(committed.current_attempt.unwrap().attempt.status, "completed");
    let detail = repository
        .get_execution_detail(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.execution.version, 5);
    assert!(detail.steps.iter().all(|step| step.status == "pending"));
}

#[tokio::test]
async fn manual_retry_clears_dispatch_gate_without_rewriting_prior_attempt() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    let attempt_conversation_id = create_conversation(&conversations, "attempt").await;
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("backoff", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let queued = repository
        .create_attempt(
            USER_ID,
            &execution_id,
            "backoff",
            0,
            None,
            &CreateAgentExecutionAttemptParams {
                participant_id: Some("participant_1".to_string()),
                start_immediately: false,
                trigger_reason: "initial".to_string(),
                effective_config: "{}".to_string(),
                retry_after: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let attempt_id = queued.current_attempt.unwrap().attempt.id;
    repository
        .start_attempt(
            USER_ID,
            &execution_id,
            "backoff",
            1,
            &attempt_id,
            0,
            attempt_conversation_id,
            None,
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let retry_after = nomifun_common::now_ms() + 60_000;
    repository
        .settle_attempt(
            USER_ID,
            &execution_id,
            "backoff",
            2,
            &attempt_id,
            1,
            None,
            &SettleAgentExecutionAttemptParams {
                attempt_status: ExecutionAttemptStatus::Failed,
                step_status: ExecutionStepStatus::Pending,
                execution_status: None,
                question: Some(None),
                error: Some(Some("transient".to_string())),
                output_summary: None,
                output_files: None,
                tokens: None,
                retry_after: Some(Some(retry_after)),
                runtime_state: None,
                started_at: None,
                finished_at: None,
                loop_repeat_reset: None,
            },
            &event(AgentExecutionEventKind::AttemptChanged),
        )
        .await
        .unwrap();
    let gated = repository
        .get_execution_detail(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gated.steps[0].dispatch_after, Some(retry_after));
    assert_eq!(gated.attempts[0].attempt.retry_after, Some(retry_after));

    let retried = repository
        .reset_steps_for_retry(
            USER_ID,
            &execution_id,
            4,
            &[RetryAgentExecutionStep {
                step_id: "backoff".to_string(),
                expected_step_version: 3,
            }],
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    assert_eq!(retried.steps[0].dispatch_after, None);
    assert_eq!(retried.attempts[0].attempt.retry_after, Some(retry_after));
}

#[tokio::test]
async fn adopt_reopens_only_the_repaired_skipped_downstream_closure() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    let mut plan = initial_plan(vec![
        agent_step("adopted", "participant_1"),
        agent_step("other_failed", "participant_1"),
        agent_step("shared", "participant_1"),
        agent_step("repaired", "participant_1"),
        agent_step("shared_child", "participant_1"),
        agent_step("unrelated", "participant_1"),
    ]);
    plan.new_dependencies = vec![
        NewAgentExecutionStepDependency {
            blocker_step_id: "adopted".to_string(),
            blocked_step_id: "shared".to_string(),
        },
        NewAgentExecutionStepDependency {
            blocker_step_id: "other_failed".to_string(),
            blocked_step_id: "shared".to_string(),
        },
        NewAgentExecutionStepDependency {
            blocker_step_id: "adopted".to_string(),
            blocked_step_id: "repaired".to_string(),
        },
        NewAgentExecutionStepDependency {
            blocker_step_id: "shared".to_string(),
            blocked_step_id: "shared_child".to_string(),
        },
    ];
    repository
        .reconcile_plan(USER_ID, &execution_id, 0, &plan, &event(AgentExecutionEventKind::PlanChanged))
        .await
        .unwrap();
    for (execution_version, step_id, status) in [
        (1, "adopted", ExecutionStepStatus::Failed),
        (2, "other_failed", ExecutionStepStatus::Failed),
        (3, "shared", ExecutionStepStatus::Skipped),
        (4, "repaired", ExecutionStepStatus::Skipped),
        (5, "shared_child", ExecutionStepStatus::Skipped),
        (6, "unrelated", ExecutionStepStatus::Skipped),
    ] {
        repository
            .transition_step_status(
                USER_ID,
                &execution_id,
                step_id,
                execution_version,
                0,
                None,
                status,
                &event(AgentExecutionEventKind::StepChanged),
            )
            .await
            .unwrap();
    }
    repository
        .adopt_step_output(
            USER_ID,
            &execution_id,
            7,
            "adopted",
            1,
            &AdoptAgentExecutionStepOutputParams {
                output_summary: "adopted output".to_string(),
                output_files: "[]".to_string(),
                tokens: None,
                runtime_state: None,
            },
            &event(AgentExecutionEventKind::StepChanged),
        )
        .await
        .unwrap();
    let detail = repository
        .get_execution_detail(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    let status = |id: &str| {
        detail
            .steps
            .iter()
            .find(|step| step.id == id)
            .unwrap()
            .status
            .as_str()
    };
    assert_eq!(status("adopted"), "completed");
    assert_eq!(status("repaired"), "pending");
    assert_eq!(status("shared"), "skipped");
    assert_eq!(status("shared_child"), "skipped");
    assert_eq!(status("unrelated"), "skipped");
}

#[tokio::test]
async fn repository_and_schema_enforce_current_dag_limit_and_step_snapshot_immutability() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "lead").await,
    )
    .await;
    let too_many = (0..=nomifun_common::MAX_AGENT_EXECUTION_STEPS)
        .map(|index| agent_step(&format!("step_{index}"), "participant_1"))
        .collect();
    assert!(
        repository
            .reconcile_plan(
                USER_ID,
                &execution_id,
                0,
                &initial_plan(too_many),
                &event(AgentExecutionEventKind::PlanChanged),
            )
            .await
            .is_err()
    );

    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &initial_plan(vec![agent_step("immutable", "participant_1")]),
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let now = nomifun_common::now_ms();
    nomifun_db::sqlx::query(
        "WITH RECURSIVE seq(i) AS ( \
             SELECT 1 UNION ALL SELECT i + 1 FROM seq WHERE i < 127 \
         ) \
         INSERT INTO agent_execution_steps ( \
             id, execution_id, title, spec, kind, agent_mode, status, \
             assigned_participant_id, assignment_source, assignment_locked, \
             failure_policy, version, introduced_in_revision, created_at, updated_at \
         ) \
         SELECT 'direct_' || i, ?, 'direct', 'direct', 'agent', 'normal', 'pending', \
                'participant_1', 'planner', 0, 'fail_execution', 0, 1, ?, ? \
         FROM seq",
    )
    .bind(&execution_id)
    .bind(now)
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();
    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO agent_execution_steps ( \
                 id, execution_id, title, spec, kind, agent_mode, status, \
                 assigned_participant_id, assignment_source, assignment_locked, \
                 failure_policy, version, introduced_in_revision, created_at, updated_at \
             ) VALUES ('overflow', ?, 'overflow', 'overflow', 'agent', 'normal', 'pending', \
                       'participant_1', 'planner', 0, 'fail_execution', 0, 1, ?, ?)",
        )
        .bind(&execution_id)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err()
    );
    assert!(
        nomifun_db::sqlx::query(
            "UPDATE agent_execution_steps SET title = 'illegal overwrite' \
             WHERE execution_id = ? AND id = 'immutable'",
        )
        .bind(&execution_id)
        .execute(database.pool())
        .await
        .is_err()
    );
}

#[tokio::test]
async fn schema_uses_plan_revision_as_the_only_graph_snapshot_clock() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let execution_id = create_execution(
        &repository,
        create_conversation(&conversations, "revision guard lead").await,
    )
    .await;
    let mut plan = initial_plan(vec![
        agent_step("revision_a", "participant_1"),
        agent_step("revision_b", "participant_1"),
    ]);
    plan.new_dependencies = vec![NewAgentExecutionStepDependency {
        blocker_step_id: "revision_a".to_owned(),
        blocked_step_id: "revision_b".to_owned(),
    }];
    repository
        .reconcile_plan(
            USER_ID,
            &execution_id,
            0,
            &plan,
            &event(AgentExecutionEventKind::PlanChanged),
        )
        .await
        .unwrap();
    let now = nomifun_common::now_ms();

    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_participants ( \
                 id, execution_id, source_agent_id, provider_id, model, \
                 enabled_skills, disabled_builtin_skills, \
                 sort_order, introduced_in_revision, created_at \
             ) VALUES ('backdated_participant', ?, 'agent', 'provider_test', 'model_test', \
                       '[]', '[]', 1, 0, ?)",
        )
        .bind(&execution_id)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "a future writer cannot invent a participant in an older revision"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_steps ( \
                 id, execution_id, title, spec, kind, agent_mode, status, \
                 assigned_participant_id, assignment_source, failure_policy, \
                 introduced_in_revision, created_at, updated_at \
             ) VALUES ('backdated_step', ?, 'backdated', '', 'agent', 'normal', 'pending', \
                       'participant_1', 'manual', 'fail_execution', 0, ?, ?)",
        )
        .bind(&execution_id)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "a future writer cannot invent a step in an older revision"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_step_dependencies ( \
                 execution_id, blocker_step_id, blocked_step_id, introduced_in_revision \
             ) VALUES (?, 'revision_a', 'revision_b', 0)",
        )
        .bind(&execution_id)
        .execute(database.pool())
        .await
        .is_err(),
        "a future writer cannot invent a dependency in an older revision"
    );
    assert!(
        sqlx::query(
            "UPDATE agent_execution_participants SET retired_in_revision = 2 \
             WHERE execution_id = ? AND id = 'participant_1'",
        )
        .bind(&execution_id)
        .execute(database.pool())
        .await
        .is_err(),
        "participant retirement must use the aggregate's current revision"
    );
    assert!(
        sqlx::query(
            "UPDATE agent_execution_steps SET superseded_in_revision = 2 \
             WHERE execution_id = ? AND id = 'revision_a'",
        )
        .bind(&execution_id)
        .execute(database.pool())
        .await
        .is_err(),
        "step supersession must use the aggregate's current revision"
    );
    assert!(
        sqlx::query(
            "UPDATE agent_execution_step_dependencies SET superseded_in_revision = 2 \
             WHERE execution_id = ? AND blocker_step_id = 'revision_a' \
               AND blocked_step_id = 'revision_b'",
        )
        .bind(&execution_id)
        .execute(database.pool())
        .await
        .is_err(),
        "dependency supersession must use the aggregate's current revision"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_attempts ( \
                 id, execution_id, step_id, attempt_no, participant_id, status, \
                 trigger_reason, effective_config, output_files, created_at, updated_at \
             ) VALUES ('skipped_generation', ?, 'revision_a', 1, 'participant_1', \
                       'queued', 'raw', '{}', '[]', ?, ?)",
        )
        .bind(&execution_id)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await
        .is_err(),
        "attempt generations must append contiguously"
    );
}

#[tokio::test]
async fn delete_is_an_atomic_tombstone_that_preserves_execution_history() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead_conversation_id = create_conversation(&conversations, "lead").await;
    let execution_id = create_execution(&repository, lead_conversation_id).await;

    assert!(
        repository
            .delete_execution(USER_ID, &execution_id, 0, &event(AgentExecutionEventKind::Deleted))
            .await
            .unwrap()
    );
    assert!(
        repository
            .get_execution(USER_ID, &execution_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repository
            .list_executions(USER_ID, 50, 0)
            .await
            .unwrap()
            .is_empty()
    );

    let raw: (String, i64, Option<i64>, i64) = sqlx::query_as(
        "SELECT status, version, deleted_at, event_sequence \
         FROM agent_executions WHERE id = ?",
    )
    .bind(&execution_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(raw.0, "cancelled");
    assert_eq!(raw.1, 1);
    assert!(raw.2.is_some());
    assert_eq!(raw.3, 2);
    let preserved_participants: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_execution_participants WHERE execution_id = ?",
    )
    .bind(&execution_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(preserved_participants, 1);
    let historical_links = repository
        .resolve_conversation_link(USER_ID, lead_conversation_id)
        .await
        .unwrap();
    assert_eq!(historical_links.len(), 1);
    assert_eq!(historical_links[0].execution_id, execution_id);
    assert!(!historical_links[0].active);
    let deleted_event: String = sqlx::query_scalar(
        "SELECT event_type FROM agent_execution_events \
         WHERE execution_id = ? ORDER BY sequence DESC LIMIT 1",
    )
    .bind(&execution_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(deleted_event, "deleted");

    assert!(
        !repository
            .delete_execution(USER_ID, &execution_id, 1, &event(AgentExecutionEventKind::Deleted))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn event_actor_is_authorized_and_on_behalf_user_is_derived_atomically() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead_conversation_id = create_conversation(&conversations, "lead").await;
    let unrelated_conversation_id = create_conversation(&conversations, "unrelated").await;
    let execution_id = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "externally initiated execution".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 4,
                work_dir: None,
                lead_conversation_id: Some(lead_conversation_id),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("participant_1")],
            &NewAgentExecutionEvent {
                event_type: AgentExecutionEventKind::Created,
                step_id: None,
                attempt_id: None,
                actor: nomifun_common::AgentExecutionActor::external_agent("companion_1"),
                payload: "{}".to_owned(),
            },
        )
        .await
        .unwrap()
        .id;

    let agent_event = NewAgentExecutionEvent {
        event_type: AgentExecutionEventKind::StatusChanged,
        step_id: None,
        attempt_id: None,
        actor: nomifun_common::AgentExecutionActor::agent(lead_conversation_id, None),
        payload: "{}".to_owned(),
    };
    repository
        .update_execution(
            USER_ID,
            &execution_id,
            0,
            None,
            &UpdateAgentExecutionParams {
                goal: Some("agent-renamed".to_owned()),
                ..Default::default()
            },
            &agent_event,
        )
        .await
        .unwrap();

    let events = repository
        .list_events(USER_ID, &execution_id, 0, 10)
        .await
        .unwrap();
    let attributed = events.last().unwrap();
    assert_eq!(attributed.actor_type, "agent");
    let expected_actor_id = lead_conversation_id.to_string();
    assert_eq!(
        attributed.actor_id.as_deref(),
        Some(expected_actor_id.as_str())
    );
    assert_eq!(attributed.actor_conversation_id, Some(lead_conversation_id));
    assert_eq!(attributed.actor_attempt_id, None);
    assert_eq!(attributed.on_behalf_of_user_id, USER_ID);

    repository
        .update_execution(
            USER_ID,
            &execution_id,
            1,
            None,
            &UpdateAgentExecutionParams {
                goal: Some("externally-renamed".to_owned()),
                ..Default::default()
            },
            &NewAgentExecutionEvent {
                event_type: AgentExecutionEventKind::StatusChanged,
                step_id: None,
                attempt_id: None,
                actor: nomifun_common::AgentExecutionActor::external_agent("companion_1"),
                payload: "{}".to_owned(),
            },
        )
        .await
        .unwrap();
    let events = repository
        .list_events(USER_ID, &execution_id, 0, 10)
        .await
        .unwrap();
    let external = events.last().unwrap();
    assert_eq!(external.actor_type, "agent");
    assert_eq!(external.actor_id.as_deref(), Some("companion_1"));
    assert_eq!(external.actor_conversation_id, None);
    assert_eq!(external.actor_attempt_id, None);
    assert_eq!(external.on_behalf_of_user_id, USER_ID);

    for actor in [
        nomifun_common::AgentExecutionActor::external_agent("companion_spoof"),
        nomifun_common::AgentExecutionActor::Agent {
            agent_id: "companion_spoof".to_owned(),
            conversation_id: Some(lead_conversation_id),
            attempt_id: None,
        },
    ] {
        let spoofed = repository
            .update_execution(
                USER_ID,
                &execution_id,
                2,
                None,
                &UpdateAgentExecutionParams {
                    goal: Some("spoofed".to_owned()),
                    ..Default::default()
                },
                &NewAgentExecutionEvent {
                    event_type: AgentExecutionEventKind::StatusChanged,
                    step_id: None,
                    attempt_id: None,
                    actor,
                    payload: "{}".to_owned(),
                },
            )
            .await;
        assert!(matches!(spoofed, Err(nomifun_db::DbError::Conflict(_))));
    }

    let denied = repository
        .update_execution(
            USER_ID,
            &execution_id,
            2,
            None,
            &UpdateAgentExecutionParams {
                goal: Some("must-roll-back".to_owned()),
                ..Default::default()
            },
            &NewAgentExecutionEvent {
                event_type: AgentExecutionEventKind::StatusChanged,
                step_id: None,
                attempt_id: None,
                actor: nomifun_common::AgentExecutionActor::agent(
                    unrelated_conversation_id,
                    None,
                ),
                payload: "{}".to_owned(),
            },
        )
        .await;
    assert!(matches!(denied, Err(nomifun_db::DbError::Conflict(_))));
    let unchanged = repository
        .get_execution(USER_ID, &execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.goal, "externally-renamed");
    assert_eq!(unchanged.version, 2);
    assert_eq!(unchanged.event_sequence, 3);
}

#[tokio::test]
async fn raw_event_writes_preserve_baseline_sequence_and_root_actor_provenance() {
    let database = test_database().await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let repository = SqliteAgentExecutionRepository::new(database.pool().clone());
    let lead_conversation_id = create_conversation(&conversations, "external event lead").await;
    let execution_id = repository
        .create_execution_with_participants(
            USER_ID,
            &CreateAgentExecutionParams {
                goal: "external baseline".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: Some(lead_conversation_id),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[participant("event_participant")],
            &NewAgentExecutionEvent {
                event_type: AgentExecutionEventKind::Created,
                step_id: None,
                attempt_id: None,
                actor: nomifun_common::AgentExecutionActor::external_agent("companion_root"),
                payload: "{}".to_owned(),
            },
        )
        .await
        .unwrap()
        .id;
    let now = nomifun_common::now_ms();

    sqlx::query("UPDATE agent_executions SET event_sequence = 2 WHERE id = ?")
        .bind(&execution_id)
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agent_execution_events ( \
             id, execution_id, sequence, event_type, actor_type, actor_id, \
             on_behalf_of_user_id, payload, created_at \
         ) VALUES ('raw_external_ok', ?, 2, 'status_changed', 'agent', 'companion_root', \
                   ?, '{}', ?)",
    )
    .bind(&execution_id)
    .bind(USER_ID)
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();

    let mut tx = database.pool().begin().await.unwrap();
    sqlx::query("UPDATE agent_executions SET event_sequence = 3 WHERE id = ?")
        .bind(&execution_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events ( \
                 id, execution_id, sequence, event_type, actor_type, actor_id, \
                 on_behalf_of_user_id, payload, created_at \
             ) VALUES ('raw_external_spoof', ?, 3, 'status_changed', 'agent', \
                       'companion_spoof', ?, '{}', ?)",
        )
        .bind(&execution_id)
        .bind(USER_ID)
        .bind(now)
        .execute(&mut *tx)
        .await
        .is_err(),
        "external events must retain the root Created actor id"
    );
    tx.rollback().await.unwrap();

    let mut tx = database.pool().begin().await.unwrap();
    sqlx::query("UPDATE agent_executions SET event_sequence = 3 WHERE id = ?")
        .bind(&execution_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events ( \
                 id, execution_id, sequence, event_type, actor_type, actor_id, \
                 actor_conversation_id, on_behalf_of_user_id, payload, created_at \
             ) VALUES ('raw_local_spoof', ?, 3, 'status_changed', 'agent', \
                       'companion_spoof', ?, ?, '{}', ?)",
        )
        .bind(&execution_id)
        .bind(lead_conversation_id)
        .bind(USER_ID)
        .bind(now)
        .execute(&mut *tx)
        .await
        .is_err(),
        "a conversation-backed Agent cannot claim a different stable id"
    );
    tx.rollback().await.unwrap();

    let mut tx = database.pool().begin().await.unwrap();
    sqlx::query(
        "INSERT INTO agent_executions ( \
             id, user_id, goal, status, plan_gate, adaptation_policy, decision_policy, \
             delegation_policy, max_parallel, initial_plan_input, event_sequence, \
             created_at, updated_at \
         ) VALUES ('bad_baseline_execution', ?, 'bad baseline', 'planning', 'automatic', \
                   'fixed', 'automatic', 'automatic', 1, '{\"mode\":\"automatic\"}', 1, ?, ?)",
    )
    .bind(USER_ID)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events ( \
                 id, execution_id, sequence, event_type, actor_type, \
                 on_behalf_of_user_id, payload, created_at \
             ) VALUES ('bad_sequence_one_kind', 'bad_baseline_execution', 1, \
                       'status_changed', 'system', ?, '{}', ?)",
        )
        .bind(USER_ID)
        .bind(now)
        .execute(&mut *tx)
        .await
        .is_err(),
        "sequence one accepts only Created or Migrated"
    );
    tx.rollback().await.unwrap();

    let mut tx = database.pool().begin().await.unwrap();
    sqlx::query("UPDATE agent_executions SET event_sequence = 3 WHERE id = ?")
        .bind(&execution_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events ( \
                 id, execution_id, sequence, event_type, actor_type, \
                 on_behalf_of_user_id, payload, created_at \
             ) VALUES ('late_created', ?, 3, 'created', 'system', ?, '{}', ?)",
        )
        .bind(&execution_id)
        .bind(USER_ID)
        .bind(now)
        .execute(&mut *tx)
        .await
        .is_err(),
        "Created cannot be written after the immutable baseline"
    );
    tx.rollback().await.unwrap();

    let mut tx = database.pool().begin().await.unwrap();
    sqlx::query(
        "INSERT INTO agent_executions ( \
             id, user_id, goal, status, plan_gate, adaptation_policy, decision_policy, \
             delegation_policy, max_parallel, initial_plan_input, event_sequence, \
             created_at, updated_at \
         ) VALUES ('raw_external_execution', ?, 'independent', 'planning', 'automatic', 'fixed', \
                   'automatic', 'automatic', 1, '{\"mode\":\"automatic\"}', 1, ?, ?)",
    )
    .bind(USER_ID)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_execution_events ( \
             id, execution_id, sequence, event_type, actor_type, actor_id, \
             on_behalf_of_user_id, payload, created_at \
         ) VALUES ('raw_independent_created', 'raw_external_execution', 1, 'created', 'agent', \
                   'companion_root', ?, '{}', ?)",
    )
    .bind(USER_ID)
    .bind(now)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE agent_executions SET event_sequence = 2 WHERE id = 'raw_external_execution'",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events ( \
                 id, execution_id, sequence, event_type, actor_type, actor_id, \
                 on_behalf_of_user_id, payload, created_at \
             ) VALUES ('raw_independent_spoof', 'raw_external_execution', 2, 'status_changed', \
                       'agent', 'companion_spoof', ?, '{}', ?)",
        )
        .bind(USER_ID)
        .bind(now)
        .execute(&mut *tx)
        .await
        .is_err(),
        "each independent Execution authorizes external events against its own initiator"
    );
    tx.rollback().await.unwrap();
}
