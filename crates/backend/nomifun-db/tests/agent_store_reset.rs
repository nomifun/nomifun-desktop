use nomifun_agent_contracts::{AGENT_STORE_BASELINE_SQL, agent_store_schema_manifest_payload};
use nomifun_db::{init_database_memory, reset_agent_data};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Executor};

#[tokio::test]
async fn agent_only_reset_removes_canonical_agent_facts_and_preserves_configuration() {
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true)
        .foreign_keys(true)
        .disable_statement_logging();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    pool.execute(AGENT_STORE_BASELINE_SQL).await.unwrap();
    pool.execute(
        r#"
        INSERT INTO schema_metadata VALUES (
            'canonical', 5, 'reset-test', 1,
            '0000000000000000000000000000000000000000000000000000000000000000',
            '1111111111111111111111111111111111111111111111111111111111111111', 1
        );
        INSERT INTO client_preferences (key, value, updated_at)
            VALUES ('theme', 'dark', 1);
        CREATE TABLE knowledge_bases (
            knowledge_base_id TEXT PRIMARY KEY,
            owner_user_id TEXT NOT NULL,
            name TEXT NOT NULL
        ) STRICT;
        INSERT INTO knowledge_bases VALUES ('knowledge-1', 'user-1', 'Reference');
        INSERT INTO mcp_servers (server_id, owner_user_id, connection_config_ref, catalog_revision)
            VALUES ('mcp-1', 'user-1', 'config-1', 1);
        INSERT INTO agent_preset_templates VALUES (
            'assistant.general', 'official', '{}',
            '2222222222222222222222222222222222222222222222222222222222222222'
        );
        INSERT INTO agent_presets VALUES (
            'preset-1', '{}', '{}', '{}', 1, 1, NULL
        );
        INSERT INTO agent_preset_revisions VALUES (
            'preset-1@1', 'preset-1', 1, '1.0.0', '{}',
            '3333333333333333333333333333333333333333333333333333333333333333',
            'user-1', 1, NULL
        );
        INSERT INTO agent_runtime_snapshots VALUES (
            'snapshot-1',
            '4444444444444444444444444444444444444444444444444444444444444444',
            '{}', '{}'
        );
        INSERT INTO agent_sessions VALUES (
            'session-1', '{"principal_kind":"user","principal_id":"user-1"}',
            'live', 'Session', 0, 0, '{}', NULL, NULL, NULL, NULL, 3, 1, NULL
        );
        INSERT INTO agent_events VALUES (
            'session-1', 1, 'event-turn', 'session-api', 'turn-idem',
            NULL, NULL, 'turn/started', 1, 'turn-1', NULL, '{}', NULL
        );
        INSERT INTO agent_events VALUES (
            'session-1', 2, 'event-effect', 'capability-host', 'effect-idem',
            NULL, NULL, 'effect/started', 1, 'effect-1', 'event-turn', '{}', NULL
        );
        INSERT INTO agent_turns VALUES (
            'session-1', 'turn-1', 'turn-1', 'turn-idem', NULL, NULL,
            'running', NULL, NULL, 'event-turn', NULL, 1, 1, NULL
        );
        INSERT INTO agent_session_resources VALUES (
            'binding-1', 'session-1', 'workspace', 'workspace-1', 'user-1',
            '["read"]', NULL, '{}',
            '5555555555555555555555555555555555555555555555555555555555555555'
        );
        INSERT INTO agent_effects VALUES (
            'effect-1', 'session-1', 'turn-1', 'effect-operation-1', 'workspace',
            'workspace.files', 'workspace.files/read', 'binding-1', 'workspace-1',
            '6666666666666666666666666666666666666666666666666666666666666666',
            'managed_effect', 'pending', NULL, 'event-effect', NULL, 2, NULL
        );
        INSERT INTO agent_session_heads VALUES (
            'session-1', 'running', 'turn-1', 0, NULL, NULL, NULL, NULL, NULL,
            NULL, 2, 0
        );
        INSERT INTO agent_messages VALUES (
            'session-1', 'message-1', 1, 2, 'assistant', '{}',
            '7777777777777777777777777777777777777777777777777777777777777777'
        );
        "#,
    )
    .await
    .unwrap();

    let report = reset_agent_data(&pool).await.unwrap();
    assert_eq!(report.preserved_rows["client_preferences"], 1);
    assert_eq!(report.preserved_rows["mcp_servers"], 1);
    assert_eq!(report.deleted_rows["agent_sessions"], 1);
    assert_eq!(report.deleted_rows["agent_effects"], 1);
    for table in [
        "agent_sessions",
        "agent_turns",
        "agent_events",
        "agent_effects",
        "agent_session_resources",
        "agent_messages",
        "agent_presets",
        "agent_preset_revisions",
        "agent_runtime_snapshots",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    let preference: String = sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'theme'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(preference, "dark");
    let knowledge_name: String = sqlx::query_scalar(
        "SELECT name FROM knowledge_bases WHERE knowledge_base_id = 'knowledge-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(knowledge_name, "Reference");
}

#[tokio::test]
async fn main_database_reset_uses_the_same_agent_generation_without_touching_users() {
    let database = init_database_memory().await.unwrap();
    let pool = database.pool();
    let users_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agent_sessions (\
            agent_session_id, owner_ref_json, state, title, archived, pinned, \
            agent_binding_json, next_seq, created_at\
         ) VALUES ('main-session', '{}', 'live', 'Main', 0, 0, '{}', 1, 1)",
    )
    .execute(pool)
    .await
    .unwrap();

    let report = reset_agent_data(pool).await.unwrap();
    assert_eq!(report.deleted_rows["agent_sessions"], 1);
    let users_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(users_after, users_before);
}

#[tokio::test]
async fn main_database_agent_store_tables_match_the_clean_baseline() {
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true)
        .foreign_keys(true)
        .disable_statement_logging();
    let baseline = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    baseline.execute(AGENT_STORE_BASELINE_SQL).await.unwrap();
    let main = init_database_memory().await.unwrap();

    for table in agent_store_schema_manifest_payload()
        .tables
        .into_iter()
        .filter(|table| table.owner == "platform.agent-session")
        .map(|table| table.table_name)
    {
        let baseline_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?",
        )
        .bind(&table)
        .fetch_one(&baseline)
        .await
        .unwrap();
        let main_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?",
        )
        .bind(&table)
        .fetch_one(main.pool())
        .await
        .unwrap();
        let normalize = |sql: &str| {
            sql.lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .flat_map(str::split_whitespace)
                .collect::<String>()
                .to_ascii_lowercase()
        };
        assert_eq!(normalize(&main_sql), normalize(&baseline_sql), "{table}");
    }
}
