use std::borrow::Cow;
use std::collections::HashSet;

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

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

async fn pool_through_041() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    migrator_through(41).run(&pool).await.unwrap();
    pool
}

async fn apply_042_like_production(pool: &SqlitePool) -> Result<(), MigrateError> {
    // Database::run_migrations sets these outside sqlx's per-migration
    // transaction. Reproduce that boundary so this test exercises the same
    // DROP/RENAME and external-FK behavior as application startup.
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA legacy_alter_table = ON")
        .execute(pool)
        .await
        .unwrap();

    let result = migrator_through(42).run(pool).await;

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA legacy_alter_table = OFF")
        .execute(pool)
        .await
        .unwrap();
    result
}

async fn seed_users(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('system_default_user', 'owner', 'hash', 1, 1), \
                ('secondary-user', 'secondary', 'hash', 1, 1)",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO providers \
         (id, platform, name, base_url, api_key_encrypted, models, \
          capabilities, created_at, updated_at) \
         VALUES ('provider-safe', 'openai', 'Safe Provider', \
                 'https://example.invalid/v1', 'encrypted', '[\"model-safe\"]', \
                 '[]', 1, 1)",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_conversation(
    pool: &SqlitePool,
    id: i64,
    user_id: &str,
    name: &str,
) {
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, model, delegation_policy, \
          decision_policy, status, created_at, updated_at) \
         VALUES (?, ?, ?, 'nomi', '{}', \
                 '{\"provider_id\":\"provider-safe\",\"model\":\"model-safe\"}', \
                 'disabled', 'automatic', 'pending', 1, 1)",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_agent_and_terminal_graph(pool: &SqlitePool) {
    seed_users(pool).await;
    seed_conversation(pool, 100, "system_default_user", "agent conversation").await;
    seed_conversation(pool, 101, "system_default_user", "terminal conversation").await;
    seed_conversation(pool, 102, "system_default_user", "artifact owner guard").await;
    seed_conversation(pool, 200, "secondary-user", "secondary conversation").await;

    sqlx::query(
        "INSERT INTO terminal_sessions \
         (id, name, cwd, command, args, created_at, updated_at, user_id) \
         VALUES (301, 'owner terminal', '/tmp', '/bin/sh', '[]', 1, 1, \
                 'system_default_user')",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, \
          payload_message, execution_mode, agent_config, conversation_id, \
          agent_type, created_by, target_kind, created_at, updated_at) \
         VALUES ('cron-agent', 'system_default_user', 'agent job', 1, 'every', \
                 '60000', 'agent work', 'existing', \
                 '{\"backend\":\"provider-safe\",\"name\":\"Nomi\",\"model_id\":\"model-safe\",\"clear_context_each_run\":false}', \
                 100, 'nomi', 'user', 'agent', 10, 10)",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, \
          payload_message, execution_mode, conversation_id, agent_type, \
          created_by, target_kind, terminal_mode, terminal_session_id, \
          terminal_command, terminal_args, terminal_script, created_at, updated_at) \
         VALUES ('cron-terminal', 'system_default_user', 'terminal job', 1, \
                 'every', '60000', 'terminal work', 'existing', 101, 'nomi', \
                 'user', 'terminal', 'existing_terminal', 301, '/bin/sh', \
                 '[]', 'echo legacy', 11, 11)",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE conversations SET cron_job_id='cron-agent' WHERE id=100")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE conversations SET cron_job_id='cron-terminal' WHERE id=101")
        .execute(pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO conversation_artifacts \
         (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
         VALUES (100, 'cron-agent', 'cron_trigger', 'active', '{}', 20, 20), \
                (101, 'cron-terminal', 'cron_trigger', 'active', '{}', 21, 21), \
                (102, 'cron-agent', 'cron_trigger', 'active', '{}', 22, 22)",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_job_runs \
         (id, job_id, executed_at_ms, status, created_at_ms) \
         VALUES ('run-agent', 'cron-agent', 30, 'ok', 30), \
                ('run-terminal', 'cron-terminal', 31, 'ok', 31)",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn expect_error_contains(pool: &SqlitePool, sql: &str, expected: &str) {
    let error = sqlx::query(sql)
        .execute(pool)
        .await
        .expect_err("statement should be rejected by a database trigger");
    let message = error.to_string();
    assert!(
        message.contains(expected),
        "expected database error containing {expected:?}, got {message:?}"
    );
}

async fn assert_external_fk(
    pool: &SqlitePool,
    child_table: &str,
    child_column: &str,
    on_delete: &str,
) {
    let sql = format!("PRAGMA foreign_key_list('{child_table}')");
    let rows = sqlx::query(&sql).fetch_all(pool).await.unwrap();
    let matching = rows.iter().find(|row| {
        row.get::<String, _>("table") == "cron_jobs"
            && row.get::<String, _>("from") == child_column
    });
    let row = matching.unwrap_or_else(|| {
        panic!("missing {child_table}.{child_column} -> cron_jobs foreign key")
    });
    assert_eq!(row.get::<String, _>("on_delete"), on_delete);
}

#[tokio::test]
async fn migration_042_rebuilds_cron_as_agent_only_without_losing_authority_guards() {
    let pool = pool_through_041().await;
    seed_agent_and_terminal_graph(&pool).await;

    apply_042_like_production(&pool).await.unwrap();

    let jobs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM cron_jobs ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(jobs, vec!["cron-agent"]);

    let agent_conversation_job: Option<String> =
        sqlx::query_scalar("SELECT cron_job_id FROM conversations WHERE id=100")
            .fetch_one(&pool)
            .await
            .unwrap();
    let terminal_conversation_job: Option<String> =
        sqlx::query_scalar("SELECT cron_job_id FROM conversations WHERE id=101")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(agent_conversation_job.as_deref(), Some("cron-agent"));
    assert!(terminal_conversation_job.is_none());

    let agent_artifact_job: Option<String> = sqlx::query_scalar(
        "SELECT cron_job_id FROM conversation_artifacts WHERE conversation_id=100",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let terminal_artifact_job: Option<String> = sqlx::query_scalar(
        "SELECT cron_job_id FROM conversation_artifacts WHERE conversation_id=101",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(agent_artifact_job.as_deref(), Some("cron-agent"));
    assert!(terminal_artifact_job.is_none());

    let runs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM cron_job_runs ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(runs, vec!["run-agent"]);
    let terminal_session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM terminal_sessions WHERE id=301")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(terminal_session_count, 1, "standalone terminals are not Cron data");

    let columns: HashSet<String> = sqlx::query("PRAGMA table_info('cron_jobs')")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert_eq!(columns.len(), 29);
    for removed in [
        "target_kind",
        "terminal_mode",
        "terminal_session_id",
        "terminal_command",
        "terminal_args",
        "terminal_script",
    ] {
        assert!(!columns.contains(removed), "legacy column survived: {removed}");
    }

    let indexes: HashSet<String> = sqlx::query(
        "SELECT name FROM sqlite_master \
         WHERE type='index' AND tbl_name='cron_jobs' AND name NOT LIKE 'sqlite_autoindex_%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get::<String, _>("name"))
    .collect();
    let expected_indexes = HashSet::from([
        "idx_cron_jobs_user".to_owned(),
        "idx_cron_jobs_conversation".to_owned(),
        "idx_cron_jobs_user_conversation".to_owned(),
        "idx_cron_jobs_next_run".to_owned(),
        "idx_cron_jobs_agent_type".to_owned(),
        "idx_cron_jobs_preset_id".to_owned(),
    ]);
    assert_eq!(indexes, expected_indexes);

    let cron_fk_parents: HashSet<String> = sqlx::query("PRAGMA foreign_key_list('cron_jobs')")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("table"))
        .collect();
    assert_eq!(
        cron_fk_parents,
        HashSet::from(["users".to_owned(), "conversations".to_owned()])
    );
    assert_external_fk(&pool, "conversations", "cron_job_id", "SET NULL").await;
    assert_external_fk(
        &pool,
        "conversation_artifacts",
        "cron_job_id",
        "SET NULL",
    )
    .await;
    assert_external_fk(&pool, "cron_job_runs", "job_id", "CASCADE").await;

    let trigger_names: HashSet<String> =
        sqlx::query("SELECT name FROM sqlite_master WHERE type='trigger'")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    let required_triggers = [
        "cron_job_owner_immutable",
        "cron_job_conversation_owner_insert",
        "cron_job_conversation_owner_update",
        "cron_execution_authority_insert_guard",
        "cron_execution_authority_update_guard",
        "conversation_cron_job_owner_insert",
        "conversation_cron_job_owner_update",
        "conversation_artifact_cron_job_owner_insert",
        "conversation_artifact_cron_job_owner_update",
        "conversation_cron_artifact_owner_immutable",
    ];
    for trigger in required_triggers {
        assert!(trigger_names.contains(trigger), "missing trigger {trigger}");
    }

    // The five triggers owned by the rebuilt table reject owner transfer,
    // cross-owner Conversation binding, and non-owner host execution.
    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, \
          payload_message, execution_mode, agent_type, created_by, created_at, updated_at) \
         VALUES ('cron-owner-move', 'system_default_user', 'owner move', 0, \
                 'every', '60000', 'x', 'new_conversation', 'nomi', 'user', 40, 40)",
    )
    .execute(&pool)
    .await
    .unwrap();
    expect_error_contains(
        &pool,
        "UPDATE cron_jobs SET user_id='secondary-user' WHERE id='cron-owner-move'",
        "cron job owner is immutable",
    )
    .await;

    expect_error_contains(
        &pool,
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, payload_message, \
          execution_mode, conversation_id, agent_type, created_by, created_at, updated_at) \
         VALUES ('cron-cross-insert', 'system_default_user', 'cross owner', 0, \
                 'every', '60000', 'x', 'existing', 200, 'nomi', 'user', 41, 41)",
        "cron job conversation owner mismatch",
    )
    .await;
    expect_error_contains(
        &pool,
        "UPDATE cron_jobs SET conversation_id=200 WHERE id='cron-agent'",
        "cron job conversation owner mismatch",
    )
    .await;

    expect_error_contains(
        &pool,
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_config, agent_type, created_by, created_at, updated_at) \
         VALUES ('cron-secondary-host', 'secondary-user', 'host config', 0, \
                 'every', '60000', 'x', 'new_conversation', \
                 '{\"backend\":\"provider-safe\",\"name\":\"Nomi\",\"workspace\":\"/\"}', \
                 'nomi', 'user', 42, 42)",
        "non-owner cron job must be model-only",
    )
    .await;
    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_config, agent_type, created_by, created_at, updated_at) \
         VALUES ('cron-secondary-safe', 'secondary-user', 'safe model', 1, \
                 'every', '60000', 'x', 'new_conversation', \
                 '{\"backend\":\"provider-safe\",\"name\":\"Nomi\",\"model_id\":\"model-safe\",\"clear_context_each_run\":false}', \
                 'nomi', 'user', 43, 43)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_type, created_by, created_at, updated_at) \
         VALUES ('cron-secondary-disabled', 'secondary-user', 'choose model', 0, \
                 'every', '60000', 'x', 'new_conversation', 'nomi', 'user', 44, 44)",
    )
    .execute(&pool)
    .await
    .unwrap();
    expect_error_contains(
        &pool,
        "UPDATE cron_jobs SET enabled=1 WHERE id='cron-secondary-disabled'",
        "non-owner cron job must be model-only",
    )
    .await;

    // The five triggers on external aggregate tables still resolve the renamed
    // cron_jobs table and enforce the same-owner relationship.
    expect_error_contains(
        &pool,
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, model, delegation_policy, decision_policy, \
          status, cron_job_id, created_at, updated_at) \
         VALUES (201, 'secondary-user', 'cross insert', 'nomi', '{}', \
                 '{\"provider_id\":\"provider-safe\",\"model\":\"model-safe\"}', \
                 'disabled', 'automatic', 'pending', 'cron-agent', 50, 50)",
        "conversation cron job owner mismatch",
    )
    .await;
    expect_error_contains(
        &pool,
        "UPDATE conversations SET cron_job_id='cron-agent' WHERE id=200",
        "conversation cron job owner mismatch",
    )
    .await;
    expect_error_contains(
        &pool,
        "INSERT INTO conversation_artifacts \
         (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
         VALUES (200, 'cron-agent', 'cron_trigger', 'active', '{}', 51, 51)",
        "conversation artifact cron job owner mismatch",
    )
    .await;
    sqlx::query(
        "INSERT INTO conversation_artifacts \
         (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
         VALUES (200, NULL, 'cron_trigger', 'active', '{}', 52, 52)",
    )
    .execute(&pool)
    .await
    .unwrap();
    expect_error_contains(
        &pool,
        "UPDATE conversation_artifacts SET cron_job_id='cron-agent' \
         WHERE conversation_id=200 AND created_at=52",
        "conversation artifact cron job owner mismatch",
    )
    .await;

    // Isolate the artifact-specific immutability trigger from migration 041's
    // broader Conversation owner guard so this assertion proves the retained
    // external trigger itself still executes after the table rebuild.
    sqlx::query("DROP TRIGGER conversation_owner_immutable")
        .execute(&pool)
        .await
        .unwrap();
    expect_error_contains(
        &pool,
        "UPDATE conversations SET user_id='secondary-user' WHERE id=102",
        "cron artifact conversation owner is immutable",
    )
    .await;

    let fk_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(fk_violations.is_empty());
}

#[tokio::test]
async fn migration_042_rejects_unknown_discriminator_atomically() {
    let pool = pool_through_041().await;
    seed_users(&pool).await;
    seed_conversation(&pool, 100, "system_default_user", "terminal conversation").await;

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, schedule_kind, schedule_value, payload_message, \
          execution_mode, conversation_id, agent_type, created_by, target_kind, \
          created_at, updated_at) \
         VALUES ('cron-terminal', 'system_default_user', 'terminal', 'every', \
                 '60000', 'x', 'existing', 100, 'nomi', 'user', 'terminal', 1, 1), \
                ('cron-unknown', 'system_default_user', 'unknown', 'every', \
                 '60000', 'x', 'new_conversation', NULL, 'nomi', 'user', 'mystery', 2, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE conversations SET cron_job_id='cron-terminal' WHERE id=100")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO conversation_artifacts \
         (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
         VALUES (100, 'cron-terminal', 'cron_trigger', 'active', '{}', 3, 3)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO cron_job_runs \
         (id, job_id, executed_at_ms, status, created_at_ms) \
         VALUES ('run-terminal', 'cron-terminal', 4, 'ok', 4)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = apply_042_like_production(&pool)
        .await
        .expect_err("unknown target_kind must abort migration 042");
    assert!(error.to_string().contains("CHECK constraint failed"));

    let applied_042: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE version=42 AND success=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(applied_042, 0);

    let columns: HashSet<String> = sqlx::query("PRAGMA table_info('cron_jobs')")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(columns.contains("target_kind"));
    assert!(columns.contains("terminal_session_id"));

    let jobs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM cron_jobs ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(jobs, vec!["cron-terminal", "cron-unknown"]);
    let conversation_job: Option<String> =
        sqlx::query_scalar("SELECT cron_job_id FROM conversations WHERE id=100")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(conversation_job.as_deref(), Some("cron-terminal"));
    let artifact_job: Option<String> =
        sqlx::query_scalar("SELECT cron_job_id FROM conversation_artifacts WHERE conversation_id=100")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(artifact_job.as_deref(), Some("cron-terminal"));
    let run_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_job_runs WHERE id='run-terminal'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(run_count, 1);

    let scratch_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type='table' AND name IN \
               ('cron_agent_only_migration_guard', 'cron_jobs_agent_only')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(scratch_tables, 0, "failed migration must roll back its scratch tables");

    let fk_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(fk_violations.is_empty());
}
