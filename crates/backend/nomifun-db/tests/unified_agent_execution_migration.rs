use std::borrow::Cow;

use sqlx::migrate::Migrator;
use sqlx::sqlite::SqlitePoolOptions;

static ALL_MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

fn migrator_through(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    }
}

async fn legacy_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrator_through(36).run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_1', 'user_1', 'hash', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for provider_id in [
        "lead",
        "new",
        "old",
        "outside",
        "p",
        "p1",
        "p2",
        "provider",
        "provider_1",
        "provider_base",
        "provider_from_snapshot",
        "provider_override",
        "provider_template",
    ] {
        sqlx::query(
            "INSERT INTO providers (\
                id, platform, name, base_url, api_key_encrypted, models, enabled, \
                capabilities, created_at, updated_at\
             ) VALUES (?, 'openai', ?, 'https://example.invalid', 'encrypted', \
                       '[]', 1, '[]', 1, 1)",
        )
        .bind(provider_id)
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool
}

#[tokio::test]
async fn migration_037_canonicalizes_bare_model_members_to_the_nomi_agent() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_bare_model', 'user_1', 'preserve a released ad-hoc model range',
             '[{"id":"bare_member","agent_id":"","provider_id":"provider_1","model":"model_1","sort_order":0}]',
             'autonomous', 'running', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, attempt, kind, override_provider_id, \
          override_model, created_at, updated_at) \
         VALUES ('bare_step', 'run_bare_model', 'step', 'execute', 'pending', 0, \
                 'agent', 'provider_override', 'override_model', 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let participants: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, source_agent_id, provider_id, model \
         FROM agent_execution_participants \
         WHERE execution_id = 'run_bare_model' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        participants,
        vec![
            (
                "bare_member".to_owned(),
                "nomi".to_owned(),
                "provider_1".to_owned(),
                "model_1".to_owned(),
            ),
            (
                "execpart_override_bare_step".to_owned(),
                "nomi".to_owned(),
                "provider_override".to_owned(),
                "override_model".to_owned(),
            ),
        ]
    );
}

#[tokio::test]
async fn migration_037_keeps_a_settled_retry_deadline_as_attempt_history_only() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_failed_retry', 'user_1', 'preserve failed retry history',
             '[{"id":"retry_member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 'failed', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES
           (10, 'user_1', 'retry 0', 'nomi',
            '{"orchestrator_run_id":"run_failed_retry","orchestrator_task_id":"failed_retry_step"}',
            'finished', 10, 11),
           (11, 'user_1', 'retry 1', 'nomi',
            '{"orchestrator_run_id":"run_failed_retry","orchestrator_task_id":"failed_retry_step"}',
            'finished', 20, 21),
           (12, 'user_1', 'retry 2', 'nomi',
            '{"orchestrator_run_id":"run_failed_retry","orchestrator_task_id":"failed_retry_step"}',
            'finished', 30, 31)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, conversation_id, attempt, kind, \
          next_retry_at, last_error, created_at, updated_at) \
         VALUES ('failed_retry_step', 'run_failed_retry', 'step', 'execute', \
                 'failed', 11, 2, 'agent', 123456, 'provider failed', 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let step: (String, Option<i64>) = sqlx::query_as(
        "SELECT status, dispatch_after FROM agent_execution_steps \
         WHERE execution_id = 'run_failed_retry' AND id = 'failed_retry_step'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(step, ("failed".to_owned(), None));

    let attempt: (
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        i64,
        i64,
    ) = sqlx::query_as(
         "SELECT status, attempt_no, retry_after, error, started_at, finished_at, \
                 created_at, updated_at \
         FROM agent_execution_attempts \
         WHERE execution_id = 'run_failed_retry' AND step_id = 'failed_retry_step' \
           AND attempt_no = 2",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        attempt,
        (
            "failed".to_owned(),
            2,
            Some(123456),
            Some("provider failed".to_owned()),
            Some(30),
            Some(31),
            30,
            31,
        )
    );
    let attempt_history: Vec<(i64, String, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT attempt_no, status, \
                json_extract(effective_config, '$.legacy_conversation_id'), \
                json_extract(effective_config, '$.legacy_conversation_source') \
         FROM agent_execution_attempts \
         WHERE execution_id = 'run_failed_retry' AND step_id = 'failed_retry_step' \
         ORDER BY attempt_no",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        attempt_history,
        vec![
            (0, "interrupted".to_owned(), Some(10), None),
            (1, "interrupted".to_owned(), Some(11), None),
            (
                2,
                "failed".to_owned(),
                Some(12),
                Some("exact_cardinality_latest".to_owned()),
            ),
        ]
    );
    let transcript_links: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT link.conversation_id, link.attempt_id, link.active \
         FROM conversation_execution_links link \
         WHERE link.execution_id = 'run_failed_retry' AND link.relation = 'attempt' \
         ORDER BY link.conversation_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        transcript_links,
        vec![
            (
                10,
                "execattempt_migrated_failed_retry_step_0".to_owned(),
                0,
            ),
            (
                11,
                "execattempt_migrated_failed_retry_step_1".to_owned(),
                0,
            ),
            (
                12,
                "execattempt_migrated_failed_retry_step_2".to_owned(),
                0,
            ),
        ]
    );
    let selection_conflict: (String, i64, i64, String) = sqlx::query_as(
        "SELECT \
             json_extract(payload, '$.legacy_conversation_selection_conflicts[0].step_id'), \
             json_extract(payload, '$.legacy_conversation_selection_conflicts[0].configured_conversation_id'), \
             json_extract(payload, '$.legacy_conversation_selection_conflicts[0].selected_conversation_id'), \
             json_extract(payload, '$.legacy_conversation_selection_conflicts[0].selection_source') \
         FROM agent_execution_events WHERE execution_id = 'run_failed_retry'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        selection_conflict,
        (
            "failed_retry_step".to_owned(),
            11,
            12,
            "exact_cardinality_latest".to_owned(),
        ),
        "complete candidate cardinality outranks a stale best-effort task column",
    );
}

#[tokio::test]
async fn migration_037_keeps_pending_retry_transcripts_historical_and_right_aligned() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_pending_gap', 'user_1', 'preserve a pending retry generation gap',
             '[{"id":"retry_member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 'running', 1, 50
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES
           (20, 'user_1', 'surviving retry 1', 'nomi',
            '{"orchestrator_run_id":"run_pending_gap","orchestrator_task_id":"pending_gap_step"}',
            'finished', 20, 21),
           (21, 'user_1', 'surviving retry 2', 'nomi',
            '{"orchestrator_run_id":"run_pending_gap","orchestrator_task_id":"pending_gap_step"}',
            'finished', 30, 31)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, conversation_id, attempt, kind, \
          next_retry_at, created_at, updated_at) \
         VALUES ('pending_gap_step', 'run_pending_gap', 'step', 'execute', \
                 'pending', 21, 3, 'agent', 123456, 1, 50)",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let attempts: Vec<(
        i64,
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT attempt_no, status, \
                json_extract(effective_config, '$.legacy_conversation_id'), \
                json_extract(effective_config, '$.legacy_candidate_ordinal'), \
                json_extract(effective_config, '$.legacy_candidate_count'), \
                json_extract(effective_config, '$.legacy_numbering') \
         FROM agent_execution_attempts \
         WHERE execution_id = 'run_pending_gap' AND step_id = 'pending_gap_step' \
         ORDER BY attempt_no",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        attempts,
        vec![
            (
                1,
                "interrupted".to_owned(),
                Some(20),
                Some(0),
                Some(2),
                Some("right_aligned".to_owned()),
            ),
            (
                2,
                "interrupted".to_owned(),
                Some(21),
                Some(1),
                Some(2),
                Some("right_aligned".to_owned()),
            ),
            (3, "queued".to_owned(), None, None, None, None),
        ],
        "surviving transcripts occupy the generations immediately before the queued retry",
    );
    let links: Vec<(i64, String)> = sqlx::query_as(
        "SELECT conversation_id, attempt_id FROM conversation_execution_links \
         WHERE execution_id = 'run_pending_gap' AND relation = 'attempt' \
         ORDER BY conversation_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        links,
        vec![
            (
                20,
                "execattempt_migrated_pending_gap_step_1".to_owned(),
            ),
            (
                21,
                "execattempt_migrated_pending_gap_step_2".to_owned(),
            ),
        ],
        "the unstarted pending generation must not inherit a stale conversation link",
    );
    let queued: (Option<i64>, String, Option<i64>) = sqlx::query_as(
        "SELECT started_at, trigger_reason, retry_after \
         FROM agent_execution_attempts \
         WHERE execution_id = 'run_pending_gap' AND step_id = 'pending_gap_step' \
           AND attempt_no = 3",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        queued,
        (None, "migrated_current_attempt".to_owned(), Some(123456))
    );
}

#[tokio::test]
async fn migration_037_rejects_a_pending_generation_zero_with_a_stale_transcript() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_impossible_pending', 'user_1', 'reject an impossible prior generation',
             '[{"id":"member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 'running', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES (
             22, 'user_1', 'impossible history', 'nomi',
             '{"orchestrator_run_id":"run_impossible_pending","orchestrator_task_id":"impossible_step"}',
             'finished', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, conversation_id, attempt, kind, \
          created_at, updated_at) \
         VALUES ('impossible_step', 'run_impossible_pending', 'step', 'execute', \
                 'pending', 22, 0, 'agent', 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = migrator_through(37).run(&pool).await.unwrap_err();
    assert!(error.to_string().contains("m037_preflight_failed"));
    let legacy_row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orch_run_tasks WHERE id = 'impossible_step'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(legacy_row_count, 1, "the failed hard cut must roll back");
}

#[tokio::test]
async fn migration_037_rejects_an_older_configured_fallback_than_surviving_history() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_ambiguous_current', 'user_1', 'reject time-travelling generations',
             '[{"id":"member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 'failed', 1, 40
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES
           (23, 'user_1', 'configured but older', 'nomi',
            '{"orchestrator_run_id":"run_ambiguous_current","orchestrator_task_id":"ambiguous_step"}',
            'finished', 10, 11),
           (24, 'user_1', 'newer surviving transcript', 'nomi',
            '{"orchestrator_run_id":"run_ambiguous_current","orchestrator_task_id":"ambiguous_step"}',
            'finished', 20, 21)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, conversation_id, attempt, kind, \
          created_at, updated_at) \
         VALUES ('ambiguous_step', 'run_ambiguous_current', 'step', 'execute', \
                 'failed', 23, 3, 'agent', 1, 40)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = migrator_through(37).run(&pool).await.unwrap_err();
    assert!(error.to_string().contains("m037_preflight_failed"));
    let source_is_intact: (i64, i64) = sqlx::query_as(
        "SELECT \
             (SELECT COUNT(*) FROM orch_run_tasks WHERE id = 'ambiguous_step'), \
             (SELECT COUNT(*) FROM conversations WHERE id IN (23, 24))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        source_is_intact,
        (1, 2),
        "ambiguous migration must roll back without rewriting either transcript",
    );
}

#[tokio::test]
async fn migration_037_preserves_a_terminal_execution_after_its_lead_was_deleted() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, lead_conv_id, status,
            created_at, updated_at)
           VALUES (
             'run_deleted_lead', 'user_1', 'preserve terminal history',
             '[{"id":"terminal_member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 404, 'completed', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let status: String = sqlx::query_scalar(
        "SELECT status FROM agent_executions WHERE id = 'run_deleted_lead'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "completed");
    let link_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_execution_links \
         WHERE execution_id = 'run_deleted_lead'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(link_count, 0);
    let archived_lead: i64 = sqlx::query_scalar(
        "SELECT json_extract(payload, '$.legacy_missing_lead_conversation_id') \
         FROM agent_execution_events WHERE execution_id = 'run_deleted_lead'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(archived_lead, 404);
}

#[tokio::test]
async fn migration_037_rejects_a_live_execution_after_its_lead_was_deleted() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, lead_conv_id, status,
            created_at, updated_at)
           VALUES (
             'run_live_deleted_lead', 'user_1', 'cannot recover without its lead',
             '[{"id":"live_member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 404, 'running', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = migrator_through(37).run(&pool).await.unwrap_err();
    assert!(error.to_string().contains("m037_preflight_failed"));
    let legacy_row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM orch_runs WHERE id = 'run_live_deleted_lead'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(legacy_row_count, 1, "the failed hard cut rolls back");
}

#[tokio::test]
async fn migration_037_rejects_non_integer_attempt_storage() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_real_attempt', 'user_1', 'reject an undecodable attempt generation',
             '[{"id":"real_member","agent_id":"nomi","provider_id":"provider_1","model":"model_1"}]',
             'autonomous', 'running', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, attempt, kind, created_at, updated_at) \
         VALUES ('real_attempt_step', 'run_real_attempt', 'step', 'execute', \
                 'pending', 0.5, 'agent', 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let storage_class: String = sqlx::query_scalar(
        "SELECT typeof(attempt) FROM orch_run_tasks WHERE id = 'real_attempt_step'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(storage_class, "real");
    let error = migrator_through(37).run(&pool).await.unwrap_err();
    assert!(error.to_string().contains("m037_preflight_failed"));
}

#[tokio::test]
async fn migration_037_preserves_execution_history_and_drops_legacy_schema() {
    let pool = legacy_pool().await;
    sqlx::query(
        "INSERT INTO requirements \
         (id, title, tag, owner_session_id, owner_kind, claimed_at, created_at, updated_at) \
         VALUES (37, 'active turn rename', 'migration', 101, 'conversation', 123456, 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES
           (1, 'user_1', 'lead', 'nomi',
            '{"agent_cluster_mode":true,"orchestrator_approval_mode":"manual","orchestrator_run_id":"run_1"}',
            'running', 1, 2),
           (2, 'user_1', 'attempt', 'nomi',
            '{"orchestrator_run_id":"run_1","orchestrator_task_id":"step_1"}',
            'running', 1, 2)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages \
         (id, conversation_id, type, content, position, status, created_at) \
         VALUES ('msg_attempt_1', 2, 'text', '{\"content\":\"working\"}', \
                 'left', 'finish', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, max_parallel, lead_conv_id,
            status, created_at, updated_at, approval_mode)
           VALUES
           ('run_1', 'user_1', 'ship',
            '[{"id":"participant_1","agent_id":"agent_1","provider_id":"provider_1","model":"model_1","preset_id":"preset_1","preset_revision":1,"preset_snapshot":{"preset_id":"preset_1","preset_revision":1,"preset_name":"Test preset","target":"cluster_member","instructions":"test"},"capability_profile":null,"constraints":{"max_concurrency":99,"cost_tier":"standard","allowed_task_kinds":["synthesis","research","agent","verify"]}}]',
            'autonomous', NULL, 1, 'running', 1, 2, 'manual'),
           ('run_fixed', 'user_1', 'resume fixed pending work',
            '[{"id":"participant_fixed","agent_id":"agent_fixed","provider_id":"provider_1","model":"model_1"}]',
            'supervised', 1, NULL, 'running', 1, 2, 'auto')"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, conversation_id, attempt, kind, \
          pending_question, output_summary, output_files, next_retry_at, created_at, updated_at) \
         VALUES ('step_1', 'run_1', 'step', 'do it', 'needs_review', 2, 3, 'agent', \
                 'approve?', 'partial', '[\"artifact.txt\"]', NULL, 1, 2), \
                ('step_control', 'run_1', 'verify', 'verify it', 'pending', NULL, 0, \
                 'verify', NULL, NULL, NULL, NULL, 1, 2), \
                ('step_fixed', 'run_fixed', 'fixed', 'resume it', 'pending', NULL, 0, \
                 'agent', NULL, NULL, NULL, 987654, 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_task_deps (blocker_task_id, blocked_task_id) \
         VALUES ('step_1', 'step_control')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_assignments \
         (id, task_id, member_id, score, rationale, source, locked, created_at) \
         VALUES ('legacy_control_assignment', 'step_control', 'participant_1', 0.25, \
                 'legacy planner routed every node', 'auto', 0, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    migrator_through(37).run(&pool).await.unwrap();

    let requirement_claim: (Option<i64>, i64) = sqlx::query_as(
        "SELECT active_turn_started_at, \
                (SELECT COUNT(*) FROM pragma_table_info('requirements') \
                  WHERE name = 'claimed_at') \
         FROM requirements WHERE id = 37",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        requirement_claim,
        (Some(123456), 0),
        "the requirement turn fence is renamed in place without duplicate legacy state"
    );

    let execution: (String, String, String, String, i64, i64, String) = sqlx::query_as(
        "SELECT status, plan_gate, adaptation_policy, decision_policy, max_parallel, event_sequence, \
                initial_plan_input \
         FROM agent_executions WHERE id = 'run_1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        execution,
        (
            "waiting_input".to_string(),
            "automatic".to_string(),
            "adaptive".to_string(),
            "ask_user".to_string(),
            4,
            1,
            r#"{"mode":"automatic"}"#.to_string(),
        )
    );
    let participant_snapshot: String = sqlx::query_scalar(
        "SELECT preset_snapshot FROM agent_execution_participants \
         WHERE execution_id = 'run_1' AND id = 'participant_1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&participant_snapshot).unwrap()["target"],
        "execution_step"
    );
    let participant_constraints: String = sqlx::query_scalar(
        "SELECT constraints FROM agent_execution_participants \
         WHERE execution_id = 'run_1' AND id = 'participant_1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let participant_constraints: serde_json::Value =
        serde_json::from_str(&participant_constraints).unwrap();
    assert!(participant_constraints.get("allowed_task_kinds").is_none());
    assert!(participant_constraints.get("cost_tier").is_none());
    assert_eq!(
        participant_constraints["allowed_profile_kinds"],
        serde_json::json!(["agent", "research", "synthesis", "verify"])
    );
    assert_eq!(participant_constraints["max_concurrency"], 64);
    let participant_capability: String = sqlx::query_scalar(
        "SELECT capability FROM agent_execution_participants \
         WHERE execution_id = 'run_1' AND id = 'participant_1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let participant_capability: serde_json::Value =
        serde_json::from_str(&participant_capability).unwrap();
    assert_eq!(participant_capability["cost_tier"], "standard");
    let attempt: (String, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT status, question, output_summary, output_files \
         FROM agent_execution_attempts WHERE execution_id = 'run_1' AND step_id = 'step_1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attempt.0, "waiting_input");
    assert_eq!(attempt.1.as_deref(), Some("approve?"));
    assert_eq!(attempt.2.as_deref(), Some("partial"));
    assert_eq!(attempt.3, r#"["artifact.txt"]"#);

    let pending_control: (String, Option<String>, String, String, Option<i64>) = sqlx::query_as(
        "SELECT step.status, step.assigned_participant_id, attempt.status, \
                attempt.trigger_reason, attempt.finished_at \
         FROM agent_execution_steps step \
         JOIN agent_execution_attempts attempt \
           ON attempt.execution_id = step.execution_id AND attempt.step_id = step.id \
         WHERE step.execution_id = 'run_1' AND step.id = 'step_control'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending_control.0, "pending");
    assert_eq!(pending_control.1, None);
    assert_eq!(pending_control.2, "cancelled");
    assert_eq!(
        pending_control.3,
        "migrated_unstarted_control_reservation"
    );
    assert_eq!(pending_control.4, Some(2));

    let fixed_pending: (String, String, String, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT execution.adaptation_policy, step.status, attempt.status, \
                step.dispatch_after, attempt.retry_after \
         FROM agent_executions execution \
         JOIN agent_execution_steps step ON step.execution_id = execution.id \
         JOIN agent_execution_attempts attempt \
           ON attempt.execution_id = step.execution_id AND attempt.step_id = step.id \
         WHERE execution.id = 'run_fixed' AND step.id = 'step_fixed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        fixed_pending,
        (
            "fixed".to_string(),
            "pending".to_string(),
            "queued".to_string(),
            Some(987654),
            Some(987654),
        ),
        "migration must preserve fixed-policy queued work and its scheduler backoff gate",
    );

    let links: Vec<(String, i64)> = sqlx::query_as(
        "SELECT relation, active FROM conversation_execution_links ORDER BY relation",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(links, vec![("attempt".to_string(), 1), ("lead".to_string(), 1)]);
    let migrated_event: (String, Option<String>, Option<i64>, Option<String>, String) =
        sqlx::query_as(
            "SELECT actor_type, actor_id, actor_conversation_id, actor_attempt_id, \
                    on_behalf_of_user_id \
             FROM agent_execution_events WHERE execution_id = 'run_1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        migrated_event,
        ("system".to_owned(), None, None, None, "user_1".to_owned())
    );
    let archived_control_assignment: (String, String, String, f64, String, String, i64, i64) =
        sqlx::query_as(
            "SELECT json_extract(archived.value, '$.id'), \
                    json_extract(archived.value, '$.task_id'), \
                    json_extract(archived.value, '$.member_id'), \
                    json_extract(archived.value, '$.score'), \
                    json_extract(archived.value, '$.rationale'), \
                    json_extract(archived.value, '$.source'), \
                    json_extract(archived.value, '$.locked'), \
                    json_extract(archived.value, '$.created_at') \
             FROM agent_execution_events event, \
                  json_each(event.payload, '$.legacy_control_assignments') archived \
             WHERE event.execution_id = 'run_1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        archived_control_assignment,
        (
            "legacy_control_assignment".to_owned(),
            "step_control".to_owned(),
            "participant_1".to_owned(),
            0.25,
            "legacy planner routed every node".to_owned(),
            "auto".to_owned(),
            0,
            1,
        )
    );
    let conversation: (String, String, String) = sqlx::query_as(
        "SELECT extra, delegation_policy, decision_policy FROM conversations WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(conversation.0, "{}");
    assert_eq!(conversation.1, "prefer_parallel");
    assert_eq!(conversation.2, "ask_user");

    let legacy_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type IN ('table', 'view') AND name IN ( \
             'fleets', 'fleet_members', 'orch_workspaces', 'orch_runs', \
             'orch_run_tasks', 'orch_run_task_deps', 'orch_assignments' \
         )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(legacy_tables, 0);
    let foreign_key_violations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_check",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(foreign_key_violations, 0);

    assert!(
        sqlx::query("DELETE FROM conversations WHERE id = 2")
            .execute(&pool)
            .await
            .is_err(),
        "attempt conversations must remain part of execution audit history"
    );
    assert!(
        sqlx::query("DELETE FROM agent_executions WHERE id = 'run_1'")
            .execute(&pool)
            .await
            .is_err(),
        "executions must use the tombstone transition"
    );
    assert!(
        sqlx::query(
            "UPDATE conversation_execution_links SET relation = 'lead' \
             WHERE relation = 'attempt'",
        )
        .execute(&pool)
        .await
        .is_err(),
        "conversation link identity must be immutable"
    );
    assert!(
        sqlx::query("DELETE FROM conversation_execution_links WHERE relation = 'lead'")
            .execute(&pool)
            .await
            .is_err(),
        "conversation links cannot be removed outside their conversation lifecycle"
    );
    assert!(
        sqlx::query("DELETE FROM conversations WHERE id = 1")
            .execute(&pool)
            .await
            .is_err(),
        "an unfinished execution must retain its authoritative lead conversation"
    );
    assert!(
        sqlx::query("UPDATE agent_executions SET status = 'completed' WHERE id = 'run_1'")
            .execute(&pool)
            .await
            .is_err(),
        "execution status transitions must be enforced by the schema"
    );
    assert!(
        sqlx::query(
            "UPDATE agent_executions SET initial_plan_input = '{\"mode\":\"explicit\",\"plan\":{\"steps\":[{}]}}' \
             WHERE id = 'run_1'",
        )
        .execute(&pool)
        .await
        .is_err(),
        "initial planning input must be immutable"
    );
    assert!(
        sqlx::query(
            "UPDATE agent_execution_attempts SET status = 'queued' \
             WHERE execution_id = 'run_1' AND step_id = 'step_1'",
        )
        .execute(&pool)
        .await
        .is_err(),
        "attempt status transitions must be enforced by the schema"
    );
    assert!(
        sqlx::query(
            "UPDATE agent_execution_attempts \
             SET status = 'interrupted', started_at = 2, finished_at = 2 \
             WHERE execution_id = 'run_fixed' AND step_id = 'step_fixed'",
        )
        .execute(&pool)
        .await
        .is_err(),
        "a queued reservation cannot claim that a concrete invocation was interrupted"
    );
    assert!(
        sqlx::query("DELETE FROM messages WHERE id = 'msg_attempt_1'")
            .execute(&pool)
            .await
            .is_err(),
        "attempt message transcripts cannot be reset, cleared, or edit-resubmitted"
    );
    assert!(
        sqlx::query("UPDATE agent_execution_events SET event_type = 'tampered'")
            .execute(&pool)
            .await
            .is_err(),
        "committed event facts must be immutable"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events (\
                 id, execution_id, sequence, event_type, actor_type, actor_id, \
                 on_behalf_of_user_id, payload, created_at\
             ) VALUES ('bad_user_actor', 'run_1', 2, 'bad', 'user', NULL, \
                       'user_1', '{}', 3)",
        )
        .execute(&pool)
        .await
        .is_err(),
        "user events require the authenticated actor id"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events (\
                 id, execution_id, sequence, event_type, actor_type, actor_id, \
                 actor_conversation_id, on_behalf_of_user_id, payload, created_at\
             ) VALUES ('bad_agent_actor', 'run_1', 2, 'bad', 'agent', '999', \
                       999, 'user_1', '{}', 3)",
        )
        .execute(&pool)
        .await
        .is_err(),
        "Agent events require one active caller link to the target execution"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events (\
                 id, execution_id, sequence, event_type, actor_type, actor_id, \
                 actor_attempt_id, on_behalf_of_user_id, payload, created_at\
             ) VALUES ('bad_external_agent_actor', 'run_1', 2, 'bad', 'agent', \
                       'companion_1', 'attempt_1', 'user_1', '{}', 3)",
        )
        .execute(&pool)
        .await
        .is_err(),
        "an external Agent event cannot claim a local attempt without a Conversation"
    );
    assert!(
        sqlx::query(
            "INSERT INTO agent_execution_events (\
                 id, execution_id, sequence, event_type, actor_type, \
                 on_behalf_of_user_id, payload, created_at\
             ) VALUES ('invented_event', 'run_1', 2, 'renamed_by_agent', 'system', \
                       'user_1', '{}', 3)",
        )
        .execute(&pool)
        .await
        .is_err(),
        "raw SQL cannot create a second execution-event vocabulary"
    );
    sqlx::query("UPDATE agent_execution_events SET published_at = 3")
    .execute(&pool)
    .await
    .unwrap();

    let audit_delete_guards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name IN ( \
             'agent_execution_participant_delete_guard', \
             'agent_execution_step_delete_guard', \
             'agent_execution_dependency_delete_guard', \
             'agent_execution_attempt_delete_guard', \
             'agent_execution_event_delete_guard' \
         )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_delete_guards, 5);

    for (table, predicate) in [
        (
            "agent_execution_participants",
            "execution_id = 'run_1' AND id = 'participant_1'",
        ),
        (
            "agent_execution_steps",
            "execution_id = 'run_1' AND id = 'step_1'",
        ),
        (
            "agent_execution_step_dependencies",
            "execution_id = 'run_1' AND blocker_step_id = 'step_1'",
        ),
        (
            "agent_execution_attempts",
            "execution_id = 'run_1' AND step_id = 'step_1'",
        ),
        ("agent_execution_events", "execution_id = 'run_1'"),
    ] {
        let statement = format!("DELETE FROM {table} WHERE {predicate}");
        assert!(
            sqlx::query(&statement).execute(&pool).await.is_err(),
            "{table} audit rows cannot be physically deleted while the owner exists",
        );
    }

    sqlx::query("UPDATE agent_executions SET status = 'failed' WHERE id = 'run_1'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM conversations WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    let terminal_lead_links: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_execution_links \
         WHERE execution_id = 'run_1' AND relation = 'lead'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        terminal_lead_links, 0,
        "a settled execution no longer blocks product deletion of its lead conversation",
    );

    // Account deletion remains the one physical lifecycle boundary and must
    // cascade Conversations and the complete Execution aggregate together.
    sqlx::query("DELETE FROM users WHERE id = 'user_1'")
        .execute(&pool)
        .await
        .unwrap();
    let execution_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_executions")
        .fetch_one(&pool)
        .await
        .unwrap();
    let conversation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
        .fetch_one(&pool)
        .await
        .unwrap();
    let child_count: i64 = sqlx::query_scalar(
        "SELECT \
             (SELECT COUNT(*) FROM agent_execution_participants) + \
             (SELECT COUNT(*) FROM agent_execution_steps) + \
             (SELECT COUNT(*) FROM agent_execution_step_dependencies) + \
             (SELECT COUNT(*) FROM agent_execution_attempts) + \
             (SELECT COUNT(*) FROM agent_execution_events) + \
             (SELECT COUNT(*) FROM messages)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((execution_count, conversation_count, child_count), (0, 0, 0));
}

#[tokio::test]
async fn migration_037_flattens_all_standalone_authoring_data_into_templates() {
    let pool = legacy_pool().await;
    sqlx::query(
        "INSERT INTO fleets \
            (id, user_id, name, description, max_parallel, created_at, updated_at) \
         VALUES \
            ('fleet_shared', 'user_1', 'Shared Fleet', 'shared description', 65, 10, 20), \
            ('fleet_empty', 'user_1', 'Empty Fleet', NULL, NULL, 11, 21)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for index in 0..66_i64 {
        let (preset_id, preset_revision, preset_snapshot, capability, constraints) =
            if index == 0 {
                (
                    Some("preset_template"),
                    Some(3_i64),
                    Some(
                        r#"{"preset_id":"preset_template","preset_revision":3,"preset_name":"Template specialist","target":"cluster_member","instructions":"work"}"#,
                    ),
                    None::<&str>,
                    Some(
                        r#"{"max_concurrency":99,"cost_tier":"premium","allowed_task_kinds":["research","agent","research"]}"#,
                    ),
                )
            } else {
                (None, None, None, None, Some("{}"))
            };
        sqlx::query(
            "INSERT INTO fleet_members (\
                id, fleet_id, agent_id, preset_id, preset_revision, preset_snapshot, \
                provider_id, model, role_hint, capability_profile, constraints, \
                sort_order, created_at, updated_at\
             ) VALUES (?, 'fleet_shared', ?, ?, ?, ?, 'provider_template', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("member_{index}"))
        .bind(format!("agent_{index}"))
        .bind(preset_id)
        .bind(preset_revision)
        .bind(preset_snapshot)
        .bind(format!("model_{index}"))
        .bind(format!("role {index}"))
        .bind(capability)
        .bind(constraints)
        .bind(index)
        .bind(100 + index)
        .bind(200 + index)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO orch_workspaces (\
            id, user_id, name, default_fleet_id, workspace_dir, context, created_at, updated_at\
         ) VALUES \
            ('ows_alpha', 'user_1', 'Alpha', 'fleet_shared', '/work/alpha', \
             '{\"scope\":\"alpha\"}', 30, 40), \
            ('ows_beta', 'user_1', 'Beta', 'fleet_shared', '/work/beta', \
             '{\"scope\":\"beta\"}', 31, 41), \
            ('ows_empty', 'user_1', 'No default', NULL, '/work/empty', \
             '{\"scope\":\"empty\"}', 32, 42)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, model, status, created_at, updated_at)
           VALUES
           (80, 'user_1', 'selected', 'nomi',
            '{"execution_template_id":"ows_alpha","keep":"yes"}',
            '{"provider_id":"provider_template","model":"model_0"}', 'pending', 1, 1),
           (81, 'user_1', 'empty selection', 'nomi',
            '{"execution_template_id":"fleet_empty"}', NULL, 'pending', 1, 1),
           (82, 'user_1', 'missing selection', 'nomi',
            '{"execution_template_id":"missing"}', NULL, 'pending', 1, 1),
           (83, 'user_1', 'mismatched lead selection', 'nomi',
            '{"execution_template_id":"ows_alpha"}',
            '{"provider_id":"outside","model":"outside"}', 'pending', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let templates: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_templates")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        templates, 3,
        "only executable Fleet and Workspace authoring aggregates become Templates"
    );
    let alpha: (String, Option<String>, Option<i64>, Option<String>, Option<String>, i64, i64) =
        sqlx::query_as(
            "SELECT name, description, max_parallel, work_dir, context, created_at, updated_at \
             FROM agent_execution_templates WHERE id = 'ows_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(alpha.0, "Alpha");
    assert_eq!(alpha.1.as_deref(), Some("shared description"));
    assert_eq!(alpha.2, Some(64));
    assert_eq!(alpha.3.as_deref(), Some("/work/alpha"));
    assert_eq!(alpha.4.as_deref(), Some(r#"{"scope":"alpha"}"#));
    assert_eq!((alpha.5, alpha.6), (30, 40));

    for template_id in ["fleet_shared", "ows_alpha", "ows_beta"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_execution_template_participants \
             WHERE template_id = ?",
        )
        .bind(template_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            count, 16,
            "legacy model pairs are retained in stable order up to the shared ceiling"
        );
    }
    for template_id in ["fleet_empty", "ows_empty"] {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_templates WHERE id = ?")
        .bind(template_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "empty authoring resources are not saved as implicit drafts");
    }

    let selections: Vec<(i64, Option<String>, String)> = sqlx::query_as(
        "SELECT id, execution_template_id, extra FROM conversations \
         WHERE id IN (80, 81, 82, 83) ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(selections[0].1.as_deref(), Some("ows_alpha"));
    assert_eq!(selections[1].1, None);
    assert_eq!(selections[2].1, None);
    assert_eq!(
        selections[3].1, None,
        "legacy selection is cleared when the concrete lead is outside the template"
    );
    for (_, _, extra) in selections {
        let extra: serde_json::Value = serde_json::from_str(&extra).unwrap();
        assert!(extra.get("execution_template_id").is_none());
    }

    let converted: (String, String, String, Option<String>, Option<String>, String, String) =
        sqlx::query_as(
            "SELECT preset_snapshot, capability, constraints, description, system_prompt, \
                    enabled_skills, disabled_builtin_skills \
             FROM agent_execution_template_participants \
             WHERE template_id = 'ows_alpha' AND id = 'member_0'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&converted.0).unwrap();
    let capability: serde_json::Value = serde_json::from_str(&converted.1).unwrap();
    let constraints: serde_json::Value = serde_json::from_str(&converted.2).unwrap();
    assert_eq!(snapshot["target"], "execution_step");
    assert_eq!(capability["cost_tier"], "premium");
    assert_eq!(
        constraints["allowed_profile_kinds"],
        serde_json::json!(["agent", "research"])
    );
    assert_eq!(constraints["max_concurrency"], 64);
    assert!(constraints.get("allowed_task_kinds").is_none());
    assert!(constraints.get("cost_tier").is_none());
    assert_eq!(converted.3, None, "missing legacy fields are not manufactured");
    assert_eq!(converted.4, None, "missing legacy fields are not manufactured");
    assert_eq!(converted.5, "[]");
    assert_eq!(converted.6, "[]");
}

#[tokio::test]
async fn migration_037_hard_cuts_conversation_model_pool_to_the_tagged_contract() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, model, extra, status, created_at, updated_at)
           VALUES
           (90, 'user_1', 'model pool', 'nomi',
            '{"provider_id":"lead","model":"lead-model","use_model":"   "}',
            '{"orchestrator_model_range":{"mode":"range","models":[{"provider_id":"p1","model":"m1"},{"provider_id":"p1","model":"m1"},{"provider_id":" p2","model":"bad"},{"provider_id":"p2","model":"m2"}]}}',
            'pending', 1, 1),
           (91, 'user_1', 'automatic model pool', 'nomi', NULL,
            '{"orchestrator_model_range":{"mode":"auto"}}', 'pending', 1, 1),
           (92, 'user_1', 'inherited model pool', 'nomi', NULL, '{}', 'pending', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let encoded: String = sqlx::query_scalar(
        "SELECT execution_model_pool FROM conversations WHERE id = 90",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
        serde_json::json!({
            "mode": "range",
            "models": [
                {"provider_id": "lead", "model": "lead-model"},
                {"provider_id": "p1", "model": "m1"},
                {"provider_id": "p2", "model": "m2"}
            ]
        })
    );
    let explicit_automatic: String = sqlx::query_scalar(
        "SELECT execution_model_pool FROM conversations WHERE id = 91",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&explicit_automatic).unwrap(),
        serde_json::json!({"mode": "automatic"}),
    );
    let inherited: Option<String> = sqlx::query_scalar(
        "SELECT execution_model_pool FROM conversations WHERE id = 92",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(inherited, None, "absence remains inherit-from-lead, not explicit automatic");

    assert!(
        sqlx::query(
            r#"UPDATE conversations SET execution_model_pool =
               '[{"provider_id":"p1","model":"m1"}]' WHERE id = 90"#,
        )
        .execute(&pool)
        .await
        .is_err(),
        "the removed bare-array alias must be rejected by SQLite",
    );
    assert!(
        sqlx::query(
            r#"UPDATE conversations SET execution_model_pool =
               '{"mode":"range","models":[{"provider_id":"p1","model":"m1"},{"provider_id":"p1","model":"m1"}]}'
               WHERE id = 90"#,
        )
        .execute(&pool)
        .await
        .is_err(),
        "duplicate tagged entries must be rejected by the DB guard",
    );
    assert!(
        sqlx::query(
            r#"UPDATE conversations SET execution_model_pool =
               '{"mode":"automatic","legacy":true}' WHERE id = 90"#,
        )
        .execute(&pool)
        .await
        .is_err(),
        "unknown tagged fields must be rejected by the DB guard",
    );
    sqlx::query(
        r#"UPDATE conversations
           SET model = '{"provider_id":"lead","model":"lead-model","use_model":""}'
           WHERE id = 90"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        sqlx::query(
            r#"UPDATE conversations
               SET model = '{"provider_id":"new","model":"new-model","use_model":"new-model"}'
               WHERE id = 90"#,
        )
        .execute(&pool)
        .await
        .is_err(),
        "repository writers cannot change a finite lead without its authority",
    );
    sqlx::query(
        r#"UPDATE conversations
           SET model = '{"provider_id":"new","model":"new-model","use_model":"new-model"}',
               execution_model_pool =
                   '{"mode":"range","models":[{"provider_id":"new","model":"new-model"},{"provider_id":"p1","model":"m1"}]}'
           WHERE id = 90"#,
    )
    .execute(&pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_037_requeues_legacy_running_work_instead_of_failing_fixed_execution() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES
           (101, 'user_1', 'lead', 'nomi',
            '{"orchestrator_run_id":"run_recover"}', 'running', 10, 20),
           (102, 'user_1', 'worker', 'nomi',
            '{"orchestrator_run_id":"run_recover","orchestrator_task_id":"step_recover"}',
            'running', 11, 21)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, lead_conv_id,
            status, created_at, updated_at)
           VALUES
           ('run_recover', 'user_1', 'resume after upgrade',
            '[{"id":"participant_recover","agent_id":"agent_recover","provider_id":"provider_1","model":"model_1"}]',
            'supervised', 101, 'running', 10, 20)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks (\
            id, run_id, title, spec, status, conversation_id, attempt, kind, \
            created_at, updated_at\
         ) VALUES (\
            'step_recover', 'run_recover', 'work', 'continue safely', 'running', \
            102, 2, 'agent', 11, 21\
         )",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();

    let state: (String, String, String, String, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT execution.status, execution.adaptation_policy, step.status, attempt.status, \
                attempt.started_at, attempt.finished_at \
         FROM agent_executions execution \
         JOIN agent_execution_steps step ON step.execution_id = execution.id \
         JOIN agent_execution_attempts attempt \
           ON attempt.execution_id = step.execution_id AND attempt.step_id = step.id \
         WHERE execution.id = 'run_recover' AND step.id = 'step_recover'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state,
        (
            "running".to_owned(),
            "fixed".to_owned(),
            "pending".to_owned(),
            "interrupted".to_owned(),
            Some(11),
            Some(21),
        ),
        "upgrade recovery must append a fresh Attempt instead of failing fixed work",
    );
    let attempt_link: (i64, Option<i64>) = sqlx::query_as(
        "SELECT active, cleanup_completed_at FROM conversation_execution_links \
         WHERE execution_id = 'run_recover' AND relation = 'attempt'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        attempt_link,
        (0, None),
        "the abandoned worker Conversation becomes durable cleanup work",
    );
}

#[tokio::test]
async fn migration_037_materializes_preset_only_runtime_models_for_recovery() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_preset_only', 'user_1', 'resume preset worker',
             '[{"id":"preset_participant","agent_id":"nomi","preset_id":"preset_1","preset_revision":2,"preset_snapshot":{"preset_id":"preset_1","preset_revision":2,"preset_name":"Preset worker","target":"cluster_member","resolved_model":{"provider_id":"provider_from_snapshot","model":"model_from_snapshot","required":true}}}]',
             'supervised', 'running', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks (\
            id, run_id, title, spec, status, attempt, kind, created_at, updated_at\
         ) VALUES (\
            'preset_step', 'run_preset_only', 'preset work', 'resume', \
            'running', 1, 'agent', 1, 2\
         )",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();
    let materialized: (Option<String>, Option<String>, String, String) = sqlx::query_as(
        "SELECT participant.provider_id, participant.model, \
                json_extract(participant.preset_snapshot, '$.target'), \
                attempt.status \
         FROM agent_execution_participants participant \
         JOIN agent_execution_steps step \
           ON step.execution_id = participant.execution_id \
          AND step.assigned_participant_id = participant.id \
         JOIN agent_execution_attempts attempt \
           ON attempt.execution_id = step.execution_id AND attempt.step_id = step.id \
         WHERE participant.execution_id = 'run_preset_only'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        materialized,
        (
            Some("provider_from_snapshot".to_owned()),
            Some("model_from_snapshot".to_owned()),
            "execution_step".to_owned(),
            "interrupted".to_owned(),
        ),
        "a recoverable runtime snapshot must carry the concrete model pair AttemptRunner requires",
    );
}

#[tokio::test]
async fn migration_037_rejects_unresolvable_nonterminal_participants_before_recovery() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_unresolvable', 'user_1', 'must remain recoverable',
             '[{"id":"missing_model","agent_id":"nomi"}]',
             'supervised', 'paused', 1, 2
           )"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let state: (i64, i64) = sqlx::query_as(
        "SELECT \
             EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'orch_runs'), \
             EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state,
        (1, 0),
        "an unrecoverable live participant rolls back Migration 037 atomically"
    );
}

#[tokio::test]
async fn migration_037_requires_live_providers_for_reopenable_but_not_cancelled_history() {
    let reopenable = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_missing_provider', 'user_1', 'reopenable history',
             '[{"id":"participant","agent_id":"nomi","provider_id":"provider_deleted","model":"model"}]',
             'supervised', 'completed', 1, 2
           )"#,
    )
    .execute(&reopenable)
    .await
    .unwrap();
    assert!(
        migrator_through(37).run(&reopenable).await.is_err(),
        "completed/failed history can reopen and therefore requires a live provider"
    );
    let rolled_back: (i64, i64) = sqlx::query_as(
        "SELECT \
             EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'orch_runs'), \
             EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&reopenable)
    .await
    .unwrap();
    assert_eq!(rolled_back, (1, 0));

    let cancelled = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES (
             'run_cancelled_provider', 'user_1', 'irreversible audit history',
             '[{"id":"participant","agent_id":"nomi","provider_id":"provider_deleted","model":"model"}]',
             'supervised', 'cancelled', 1, 2
           )"#,
    )
    .execute(&cancelled)
    .await
    .unwrap();
    migrator_through(37).run(&cancelled).await.unwrap();
    let preserved: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT execution.status, participant.provider_id, participant.model \
         FROM agent_executions execution \
         JOIN agent_execution_participants participant \
           ON participant.execution_id = execution.id \
         WHERE execution.id = 'run_cancelled_provider'",
    )
    .fetch_one(&cancelled)
    .await
    .unwrap();
    assert_eq!(
        preserved,
        (
            "cancelled".to_owned(),
            Some("provider_deleted".to_owned()),
            Some("model".to_owned()),
        ),
        "Migration may preserve non-executable provider identity only in irreversible Cancelled audit history"
    );
}

#[tokio::test]
async fn migration_037_separates_free_form_roles_from_tool_authority() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES ('run_roles', 'user_1', 'preserve roles',
                   '[{"id":"role_participant","agent_id":"nomi","provider_id":"provider","model":"model"}]',
                   'supervised', 'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_run_tasks
           (id, run_id, title, spec, role, status, attempt, kind, created_at, updated_at)
           VALUES
             ('role_1', 'run_roles', 'one', 'one', 'Searcher', 'pending', 0, 'agent', 1, 1),
             ('role_2', 'run_roles', 'two', 'two', 'reviewer', 'pending', 0, 'agent', 1, 1),
             ('role_3', 'run_roles', 'three', 'three', 'tester', 'pending', 0, 'agent', 1, 1),
             ('role_4', 'run_roles', 'four', 'four', '验证负责人', 'pending', 0, 'agent', 1, 1),
             ('role_5', 'run_roles', 'five', 'five', 'custom-role', 'pending', 0, 'agent', 1, 1),
             ('role_6', 'run_roles', 'six', 'six', NULL, 'pending', 0, 'agent', 1, 1),
             ('role_7', 'run_roles', 'seven', 'seven', 'searcher', 'pending', 0, 'verify', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_task_deps (blocker_task_id, blocked_task_id) \
         VALUES ('role_6', 'role_7')",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();
    let policies: Vec<(Option<String>, String)> = sqlx::query_as(
        "SELECT role, tool_policy FROM agent_execution_steps \
         WHERE execution_id = 'run_roles' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        policies,
        vec![
            (Some("Searcher".to_owned()), "read_only".to_owned()),
            (Some("reviewer".to_owned()), "read_only".to_owned()),
            (Some("tester".to_owned()), "read_shell".to_owned()),
            (Some("验证负责人".to_owned()), "full".to_owned()),
            (Some("custom-role".to_owned()), "full".to_owned()),
            (None, "full".to_owned()),
            (Some("searcher".to_owned()), "full".to_owned()),
        ],
        "legacy compatibility narrows only known aliases; role remains descriptive data",
    );
}

#[tokio::test]
async fn migration_037_rejects_malformed_standalone_template_data_transactionally() {
    let pool = legacy_pool().await;
    sqlx::query(
        "INSERT INTO fleets \
            (id, user_id, name, description, created_at, updated_at) \
         VALUES ('fleet_bad', 'user_1', 'Bad Fleet', NULL, 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fleet_members (\
            id, fleet_id, agent_id, provider_id, model, constraints, \
            sort_order, created_at, updated_at\
         ) VALUES (\
            'member_bad', 'fleet_bad', 'agent_1', 'provider_1', 'model_1', \
            '{not-json', 0, 1, 2\
         )",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let legacy_still_online: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM fleets WHERE id = 'fleet_bad'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(legacy_still_online, 1);
    let target_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_execution_templates'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(target_exists, 0, "failed migration must roll back the target schema");
}

#[tokio::test]
async fn migration_037_canonicalizes_collaboration_model_preference_once() {
    let legacy_only = legacy_pool().await;
    sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) \
         VALUES ('nomi.orchestrationCollaborators', '[{\"provider_id\":\"old\",\"model\":\"m\"}]', 7)",
    )
    .execute(&legacy_only)
    .await
    .unwrap();
    migrator_through(37).run(&legacy_only).await.unwrap();
    // A second Migrator run is the real startup idempotency path: version 37
    // is already recorded and must leave the canonical state untouched.
    migrator_through(37).run(&legacy_only).await.unwrap();
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT key, value, updated_at FROM client_preferences \
         WHERE key IN ('nomi.orchestrationCollaborators', \
                       'nomi.executionCollaborators', 'nomi.collaborationModels') \
         ORDER BY key",
    )
    .fetch_all(&legacy_only)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![(
            "nomi.collaborationModels".to_owned(),
            r#"[{"provider_id":"old","model":"m"}]"#.to_owned(),
            7,
        )]
    );

    let intermediate_wins = legacy_pool().await;
    sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) VALUES \
         ('nomi.orchestrationCollaborators', '[\"released-old\"]', 6), \
         ('nomi.executionCollaborators', '[\"intermediate\"]', 8)",
    )
    .execute(&intermediate_wins)
    .await
    .unwrap();
    migrator_through(37).run(&intermediate_wins).await.unwrap();
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT key, value, updated_at FROM client_preferences \
         WHERE key IN ('nomi.orchestrationCollaborators', \
                       'nomi.executionCollaborators', 'nomi.collaborationModels') \
         ORDER BY key",
    )
    .fetch_all(&intermediate_wins)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![(
            "nomi.collaborationModels".to_owned(),
            r#"["intermediate"]"#.to_owned(),
            8,
        )]
    );

    let canonical_wins = legacy_pool().await;
    sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) VALUES \
         ('nomi.orchestrationCollaborators', '[\"released-old\"]', 6), \
         ('nomi.executionCollaborators', '[\"intermediate\"]', 7), \
         ('nomi.collaborationModels', '[\"canonical\"]', 9)",
    )
    .execute(&canonical_wins)
    .await
    .unwrap();
    migrator_through(37).run(&canonical_wins).await.unwrap();
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT key, value, updated_at FROM client_preferences \
         WHERE key IN ('nomi.orchestrationCollaborators', \
                       'nomi.executionCollaborators', 'nomi.collaborationModels') \
         ORDER BY key",
    )
    .fetch_all(&canonical_wins)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![(
            "nomi.collaborationModels".to_owned(),
            r#"["canonical"]"#.to_owned(),
            9,
        )]
    );
}

#[tokio::test]
async fn migration_037_rejects_a_legacy_dependency_cycle_transactionally() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES
           ('run_1', 'user_1', 'ship',
            '[{"id":"participant_1","agent_id":"agent_1","provider_id":"provider_1","model":"model_1"}]',
            'supervised', 'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks \
         (id, run_id, title, spec, status, attempt, kind, created_at, updated_at) \
         VALUES ('step_1', 'run_1', 'one', 'one', 'pending', 0, 'agent', 1, 1), \
                ('step_2', 'run_1', 'two', 'two', 'pending', 0, 'agent', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_task_deps (blocker_task_id, blocked_task_id) \
         VALUES ('step_1', 'step_2'), ('step_2', 'step_1')",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let legacy_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'orch_runs')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let unified_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(legacy_table_exists, 1);
    assert_eq!(unified_table_exists, 0);
}

#[tokio::test]
async fn migration_037_rejects_waiting_attempt_conversation_that_is_also_a_lead() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES (41, 'user_1', 'ambiguous active actor', 'nomi', '{}',
                   'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, lead_conv_id, status,
            created_at, updated_at)
           VALUES
           ('attempt_owner', 'user_1', 'waiting',
            '[{"id":"p1","agent_id":"a1"}]', 'supervised', NULL, 'running', 1, 1),
           ('lead_owner', 'user_1', 'lead',
            '[{"id":"p2","agent_id":"a2"}]', 'supervised', 41, 'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_run_tasks
           (id, run_id, title, spec, status, conversation_id, pending_question,
            attempt, kind, created_at, updated_at)
           VALUES ('waiting_step', 'attempt_owner', 'waiting', 'wait',
                   'needs_review', 41, 'approve?', 0, 'agent', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let source_intact: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'orch_runs')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let target_absent: i64 = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((source_intact, target_absent), (1, 1));
}

#[tokio::test]
async fn migration_037_rejects_a_legacy_execution_parent_cycle_transactionally() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, forked_from,
            created_at, updated_at)
           VALUES
           ('cycle_1', 'user_1', 'one', '[{"id":"p1","agent_id":"a1"}]',
            'supervised', 'running', NULL, 1, 1),
           ('cycle_2', 'user_1', 'two', '[{"id":"p2","agent_id":"a2"}]',
            'supervised', 'running', 'cycle_1', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE orch_runs SET forked_from = 'cycle_2' WHERE id = 'cycle_1'")
        .execute(&pool)
        .await
        .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let legacy_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'orch_runs')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let unified_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((legacy_table_exists, unified_table_exists), (1, 0));
}

#[tokio::test]
async fn migration_037_rejects_unreadable_participant_snapshots_transactionally() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at)
           VALUES
           ('run_1', 'user_1', 'ship',
            '[{"id":"participant_1","agent_id":"agent_1","provider_id":"provider_1","model":"model_1","capability_profile":{"strengths":"not-an-array","modalities":[],"tools":true,"reasoning":"medium","cost_tier":"standard","speed_tier":"standard"}}]',
            'supervised', 'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let legacy_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'orch_runs')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let unified_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((legacy_table_exists, unified_table_exists), (1, 0));
}

#[tokio::test]
async fn migration_037_preserves_effective_delegation_depth_from_parent_steps() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO conversations
           (id, user_id, name, type, extra, status, created_at, updated_at)
           VALUES
           (1, 'user_1', 'root lead', 'nomi', '{}', 'running', 1, 1),
           (2, 'user_1', 'root attempt and child lead', 'nomi',
            '{"orchestrator_run_id":"root","orchestrator_task_id":"root_step"}',
            'running', 1, 1),
           (3, 'user_1', 'child attempt and grandchild lead', 'nomi',
            '{"orchestrator_run_id":"child","orchestrator_task_id":"child_step"}',
            'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, lead_conv_id, status,
            forked_from, created_at, updated_at)
           VALUES
           ('root', 'user_1', 'root',
            '[{"id":"root_p","agent_id":"a","provider_id":"provider","model":"model"}]',
            'supervised', 1, 'running', NULL, 1, 1),
           ('child', 'user_1', 'child',
            '[{"id":"child_p","agent_id":"a","provider_id":"provider","model":"model"}]',
            'supervised', 2, 'running', 'root', 1, 1),
           ('grandchild', 'user_1', 'grandchild',
            '[{"id":"grandchild_p","agent_id":"a","provider_id":"provider","model":"model"}]',
            'supervised', 3, 'running', 'child', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_run_tasks
           (id, run_id, title, spec, status, conversation_id, attempt, kind,
            pattern_config, created_at, updated_at)
           VALUES
           ('root_step', 'root', 'root step', 'delegate', 'running', 2, 1, 'agent',
            '{"delegation_depth":2}', 1, 1),
           ('child_step', 'child', 'child step', 'delegate', 'running', 3, 1, 'agent',
            '{"delegation_depth":0}', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();
    let depths: Vec<(String, i64)> = sqlx::query_as(
        "SELECT id, delegation_depth FROM agent_execution_steps ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        depths,
        vec![
            ("child_step".to_owned(), 3),
            ("root_step".to_owned(), 2),
        ]
    );
    let execution_tree_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('agent_executions') \
         WHERE name IN ('parent_execution_id', 'delegation_depth')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(execution_tree_columns, 0, "runtime Executions are independent aggregates");
    let step_depth_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('agent_execution_steps') \
         WHERE name = 'delegation_depth'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        step_depth_columns, 1,
        "the effective legacy recursion budget is reduced to one private Step fact"
    );
    let fork_audit: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT execution_id, json_extract(payload, '$.legacy_forked_from') \
         FROM agent_execution_events ORDER BY execution_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        fork_audit,
        vec![
            ("child".to_owned(), Some("root".to_owned())),
            ("grandchild".to_owned(), Some("child".to_owned())),
            ("root".to_owned(), None),
        ]
    );
}

#[tokio::test]
async fn migration_037_rejects_parallelism_above_the_shared_ceiling() {
    let pool = legacy_pool().await;
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, max_parallel, status,
            created_at, updated_at)
           VALUES ('too_wide', 'user_1', 'ship',
                   '[{"id":"p","agent_id":"a"}]',
                   'supervised', 65, 'running', 1, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
}

#[tokio::test]
async fn migration_037_rejects_legacy_current_participants_above_shared_ceiling() {
    let pool = legacy_pool().await;
    let snapshot = serde_json::to_string(
        &(0..64)
            .map(|index| {
                serde_json::json!({
                    "id": format!("participant_{index}"),
                    "agent_id": "agent_nomi",
                    "provider_id": "provider_base",
                    "model": "model_base",
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_runs ( \
             id, user_id, goal, fleet_snapshot, autonomy, status, created_at, updated_at \
         ) VALUES ('too_many_participants', 'user_1', 'ship', ?, 'supervised', \
                   'running', 1, 1)",
    )
    .bind(snapshot)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orch_run_tasks ( \
             id, run_id, title, spec, status, attempt, kind, \
             override_provider_id, override_model, created_at, updated_at \
         ) VALUES ('override_step', 'too_many_participants', 'step', 'work', \
                   'pending', 0, 'agent', 'provider_override', 'model_override', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(migrator_through(37).run(&pool).await.is_err());
    let legacy_execution: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM orch_runs WHERE id = 'too_many_participants'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let unified_schema: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_executions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((legacy_execution, unified_schema), (1, 0));
}

#[tokio::test]
async fn migration_037_preserves_shared_lead_history_and_selects_one_current_execution() {
    let pool = legacy_pool().await;
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, created_at, updated_at) \
         VALUES (41, 'user_1', 'ambiguous lead', 'nomi', '{}', 'running', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO orch_runs
           (id, user_id, goal, fleet_snapshot, autonomy, lead_conv_id,
            status, created_at, updated_at)
           VALUES
           ('run_lead_a', 'user_1', 'a',
            '[{"id":"participant_a","agent_id":"agent_a","provider_id":"p","model":"m"}]',
            'supervised', 41, 'running', 1, 2),
           ('run_lead_b', 'user_1', 'b',
            '[{"id":"participant_b","agent_id":"agent_b","provider_id":"p","model":"m"}]',
            'supervised', 41, 'failed', 1, 9),
           ('run_lead_c', 'user_1', 'c',
            '[{"id":"participant_c","agent_id":"agent_c","provider_id":"p","model":"m"}]',
            'supervised', 41, 'planning', 1, 3)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(37).run(&pool).await.unwrap();
    let links: Vec<(String, i64)> = sqlx::query_as(
        "SELECT execution_id, active FROM conversation_execution_links \
         WHERE conversation_id = 41 AND relation = 'lead' ORDER BY execution_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        links,
        vec![
            ("run_lead_a".to_owned(), 0),
            ("run_lead_b".to_owned(), 0),
            ("run_lead_c".to_owned(), 1),
        ],
        "unfinished rows outrank terminal history, then newest updated_at is current"
    );
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_execution_links \
         WHERE conversation_id = 41 AND relation = 'lead' AND active = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active_count, 1);
}

#[tokio::test]
async fn migration_037_installs_durable_conversation_receipt_guards() {
    let pool = legacy_pool().await;
    migrator_through(37).run(&pool).await.unwrap();

    let receipt_table: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'conversation_delivery_receipts')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(receipt_table, 1);

    let triggers: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'trigger' \
         AND name IN ('conversation_delivery_receipt_update_guard', \
                      'conversation_delivery_receipt_delete_guard') ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        triggers,
        vec![
            "conversation_delivery_receipt_delete_guard".to_owned(),
            "conversation_delivery_receipt_update_guard".to_owned(),
        ]
    );
}
