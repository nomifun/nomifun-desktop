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

#[tokio::test]
async fn migration_041_hard_cuts_secondary_users_to_model_only_execution() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    migrator_through(40).run(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('system_default_user', 'owner', 'hash', 1, 1), \
                ('secondary-user', 'secondary-user', 'hash', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, delegation_policy, decision_policy, status, created_at, updated_at) \
         VALUES (400, 'system_default_user', 'owner legacy capability', 'nomi', \
                 '{\"desktopGateway\":true,\"gateway_mcp_config\":{\"token\":\"root\"},\"workspace\":\"/work\"}', \
                 'automatic', 'automatic', 'pending', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, delegation_policy, execution_model_pool, \
          decision_policy, channel_chat_id, status, created_at, updated_at) \
         VALUES (401, 'secondary-user', 'unsafe', 'acp', \
                 '{\"workspace\":\"/\",\"desktopGateway\":true}', \
                 'prefer_parallel', '{\"mode\":\"automatic\"}', \
                 'ask_user', 'remote-chat', 'pending', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO terminal_sessions \
         (id, name, cwd, command, args, created_at, updated_at, user_id) \
         VALUES (402, 'unsafe terminal', '/', '/bin/sh', '[]', 1, 1, 'secondary-user'), \
                (403, 'owner terminal', '/', '/bin/sh', '[]', 1, 1, 'system_default_user')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_config, agent_type, created_by, target_kind, created_at, updated_at) \
         VALUES ('cron-secondary', 'secondary-user', 'unsafe cron', 'every', '60000', 'work', \
                 'new_conversation', '{\"backend\":\"claude\"}', 'acp', 'user', 'agent', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_config, agent_type, created_by, target_kind, \
          terminal_session_id, skill_content, preset_id, preset_revision, \
          preset_snapshot, created_at, updated_at) \
         VALUES ('cron-secondary-safe-model', 'secondary-user', 'safe model cron', \
                 'every', '60000', 'work', 'new_conversation', \
                 '{\"backend\":\"provider-safe\",\"name\":\"Nomi\",\"model_id\":\"model-safe\",\"workspace\":\"/unsafe\",\"mode\":\"yolo\",\"clear_context_each_run\":true}', \
                 'nomi', 'user', 'terminal', 402, 'legacy host skill', \
                 'legacy-preset', 7, '{\"id\":\"forged\"}', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agent_executions \
         (id, user_id, goal, status, plan_gate, adaptation_policy, decision_policy, \
          delegation_policy, max_parallel, initial_plan_input, created_at, updated_at) \
         VALUES ('exec-secondary', 'secondary-user', 'unsafe execution', 'running', \
                 'automatic', 'fixed', 'automatic', 'automatic', 1, \
                 '{\"mode\":\"automatic\"}', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(41).run(&pool).await.unwrap();

    let owner_extra: String =
        sqlx::query_scalar("SELECT extra FROM conversations WHERE id=400")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&owner_extra).unwrap(),
        serde_json::json!({ "workspace": "/work" })
    );

    let conversation: (String, String, String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT type, extra, delegation_policy, execution_model_pool, channel_chat_id \
         FROM conversations WHERE id=401",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        conversation,
        ("nomi".into(), "{}".into(), "disabled".into(), None, None)
    );

    let terminal_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM terminal_sessions WHERE id=402")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(terminal_exists, 0);

    let move_owner_conversation =
        sqlx::query("UPDATE conversations SET user_id='secondary-user' WHERE id=400")
            .execute(&pool)
            .await;
    assert!(
        move_owner_conversation.is_err(),
        "a host-created Conversation cannot be reassigned into model-only authority"
    );

    let move_owner_terminal =
        sqlx::query("UPDATE terminal_sessions SET user_id='secondary-user' WHERE id=403")
            .execute(&pool)
            .await;
    assert!(
        move_owner_terminal.is_err(),
        "a live host process cannot be reassigned into model-only authority"
    );

    let cron: (String, Option<String>, bool, Option<String>) = sqlx::query_as(
        "SELECT agent_type, agent_config, enabled, last_error \
         FROM cron_jobs WHERE id='cron-secondary'",
    )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(cron.0, "nomi");
    assert!(cron.1.is_none());
    assert!(!cron.2, "an unbound legacy ACP schedule has no safe Nomi model");
    assert!(cron.3.as_deref().is_some_and(|message| message.contains("choose a Nomi model")));

    let safe_cron: (
        String,
        Option<String>,
        bool,
        Option<String>,
        Option<String>,
        String,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT agent_type, agent_config, enabled, preset_id, skill_content, \
                target_kind, terminal_session_id \
         FROM cron_jobs WHERE id='cron-secondary-safe-model'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(safe_cron.0, "nomi");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(safe_cron.1.as_deref().unwrap()).unwrap(),
        serde_json::json!({
            "backend": "provider-safe",
            "name": "Nomi",
            "model_id": "model-safe",
            "clear_context_each_run": true,
        })
    );
    assert!(safe_cron.2, "safe model selection must keep the schedule enabled");
    assert!(safe_cron.3.is_none() && safe_cron.4.is_none());
    assert_eq!(safe_cron.5, "agent");
    assert!(safe_cron.6.is_none());

    let deleted_at: Option<i64> =
        sqlx::query_scalar("SELECT deleted_at FROM agent_executions WHERE id='exec-secondary'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(deleted_at.is_some(), "secondary execution audit must be tombstoned");

    let unsafe_conversation = sqlx::query(
        "INSERT INTO conversations \
         (user_id, name, type, extra, status, created_at, updated_at) \
         VALUES ('secondary-user', 'forged', 'acp', '{}', 'pending', 2, 2)",
    )
    .execute(&pool)
    .await;
    assert!(unsafe_conversation.is_err());

    sqlx::query(
        "INSERT INTO conversations \
         (user_id, name, type, extra, delegation_policy, status, created_at, updated_at) \
         VALUES ('secondary-user', 'safe', 'nomi', '{}', 'disabled', 'pending', 2, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let unsafe_cron = sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_type, created_by, target_kind, created_at, updated_at) \
         VALUES ('cron-forged', 'secondary-user', 'forged', 'every', '60000', 'x', \
                 'new_conversation', 'acp', 'user', 'agent', 2, 2)",
    )
    .execute(&pool)
    .await;
    assert!(unsafe_cron.is_err());

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_config, agent_type, created_by, target_kind, created_at, updated_at) \
         VALUES ('cron-model-only', 'secondary-user', 'safe', 'every', '60000', 'x', \
                 'new_conversation', \
                 '{\"backend\":\"provider-safe\",\"name\":\"Nomi\",\"model_id\":\"model-safe\",\"clear_context_each_run\":false}', \
                 'nomi', 'user', 'agent', 2, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_jobs \
         (id, user_id, name, enabled, schedule_kind, schedule_value, payload_message, \
          execution_mode, agent_type, created_by, target_kind, created_at, updated_at) \
         VALUES ('cron-disabled-no-model', 'secondary-user', 'choose a model', 0, \
                 'every', '60000', 'x', 'new_conversation', 'nomi', 'user', 'agent', 2, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let reenable_without_model =
        sqlx::query("UPDATE cron_jobs SET enabled=1 WHERE id='cron-disabled-no-model'")
            .execute(&pool)
            .await;
    assert!(
        reenable_without_model.is_err(),
        "a disabled migrated schedule cannot resume without selecting a Nomi model"
    );

    let unsafe_cron_config = sqlx::query(
        "UPDATE cron_jobs \
         SET agent_config='{\"backend\":\"provider-safe\",\"name\":\"Nomi\",\"workspace\":\"/\"}' \
         WHERE id='cron-model-only'",
    )
    .execute(&pool)
    .await;
    assert!(
        unsafe_cron_config.is_err(),
        "the model-only config cannot grow a host workspace field"
    );

    let unsafe_cron_skill = sqlx::query(
        "UPDATE cron_jobs SET skill_content='host instructions' WHERE id='cron-model-only'",
    )
    .execute(&pool)
    .await;
    assert!(unsafe_cron_skill.is_err(), "secondary cron cannot persist a host skill");

    let unsafe_terminal = sqlx::query(
        "INSERT INTO terminal_sessions \
         (name, cwd, command, args, created_at, updated_at, user_id) \
         VALUES ('forged', '/', '/bin/sh', '[]', 2, 2, 'secondary-user')",
    )
    .execute(&pool)
    .await;
    assert!(unsafe_terminal.is_err());

    let unsafe_execution = sqlx::query(
        "INSERT INTO agent_executions \
         (id, user_id, goal, status, plan_gate, adaptation_policy, decision_policy, \
          delegation_policy, max_parallel, initial_plan_input, created_at, updated_at) \
         VALUES ('exec-forged', 'secondary-user', 'forged', 'planning', 'automatic', \
                 'fixed', 'automatic', 'disabled', 1, '{\"mode\":\"automatic\"}', 2, 2)",
    )
    .execute(&pool)
    .await;
    assert!(unsafe_execution.is_err());
}
