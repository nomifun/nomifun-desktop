use nomifun_agent_contracts::{AGENT_STORE_BASELINE_SQL, agent_store_schema_manifest_payload};
use nomifun_db::{init_database_memory, reset_agent_data};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Executor};

#[tokio::test]
async fn main_database_reset_uses_the_same_agent_generation_without_touching_users() {
    let database = init_database_memory().await.unwrap();
    let pool = database.pool();
    let session_id = "0190f5fe-7c00-7a00-8000-000000000101";
    let binding_id = "0190f5fe-7c00-7a00-8000-000000000102";
    let event_id = "0190f5fe-7c00-7a00-8000-000000000103";
    let owner = nomifun_db::installation_owner_id(pool).await.unwrap();
    let users_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agent_sessions (\
            agent_session_id, owner_ref_json, state, title, archived, pinned, \
            agent_binding_json, next_seq, created_at\
         ) VALUES (?, '{}', 'live', 'Main', 0, 0, '{}', 1, 1)",
    )
    .bind(session_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO remote_bindings (\
            remote_binding_id, owner_user_id, name, agent_binding_json, \
            nomi_snapshot_json, provenance_json, agent_binding_digest, \
            binding_version, created_at, updated_at\
         ) VALUES (?, ?, 'Reset remote', '{}', '{}', '{}', ?, 1, 1, 1)",
    )
    .bind(binding_id)
    .bind(&owner)
    .bind("a".repeat(64))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_remote_sessions (\
            agent_session_id, owner_user_id, remote_binding_id, open_idempotency_key, \
            binding_version, agent_binding_digest, agent_binding_json, \
            nomi_snapshot_json, provenance_json, state, created_at, updated_at\
         ) VALUES (?, ?, ?, 'open', 1, ?, '{}', '{}', '{}', 'ready', 1, 1)",
    )
    .bind(session_id)
    .bind(&owner)
    .bind(binding_id)
    .bind("a".repeat(64))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_remote_events (\
            event_id, agent_session_id, seq, event_type, payload_json, created_at\
         ) VALUES (?, ?, 1, 'opened', '{}', 1)",
    )
    .bind(event_id)
    .bind(session_id)
    .execute(pool)
    .await
    .unwrap();

    let report = reset_agent_data(pool).await.unwrap();
    assert_eq!(report.deleted_rows["agent_sessions"], 1);
    assert_eq!(report.deleted_rows["nomi_remote_events"], 1);
    assert_eq!(report.deleted_rows["nomi_remote_sessions"], 1);
    let users_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(users_after, users_before);
}

#[tokio::test]
async fn main_database_agent_store_tables_match_the_forward_migrated_baseline() {
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
    for migration in [
        include_str!("../migrations/007_retire_terminal_content_mirrors.sql"),
        include_str!("../migrations/008_retire_payload_object_placeholder.sql"),
        include_str!("../migrations/009_cron_input_handoff.sql"),
        include_str!("../migrations/010_mcp_oauth_authority.sql"),
        include_str!("../migrations/011_cron_run_cancellation.sql"),
        include_str!("../migrations/012_unify_cron_agent_binding.sql"),
        include_str!("../migrations/015_retire_execution_plan_gate.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&baseline).await.unwrap();
    }
    let main = init_database_memory().await.unwrap();

    for table in agent_store_schema_manifest_payload()
        .tables
        .into_iter()
        .filter(|table| table.owner == "platform.agent-session")
        .map(|table| table.table_name)
    {
        let baseline_sql: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
                .bind(&table)
                .fetch_one(&baseline)
                .await
                .unwrap();
        let main_sql: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
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

#[tokio::test]
async fn reset_preserves_cron_definitions_and_author_model_but_retires_agent_authority() {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool();
    let owner = nomifun_db::installation_owner_id(pool).await.unwrap();
    let provider = "0190f5fe-7c00-7a00-8000-000000000141";
    let job = "0190f5fe-7c00-7a00-8000-000000000142";
    sqlx::query("INSERT INTO providers(provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,created_at,updated_at) VALUES(?,'openai','Preserve model','https://example.invalid','bearer','fixture',1,1,1)").bind(provider).execute(pool).await.unwrap();
    let model = serde_json::json!({"provider_id":provider,"model":"keep-model"}).to_string();
    sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,enabled,schedule_kind,schedule_value,payload_message,agent_selection,model_selection,next_run_at,created_by,created_at,updated_at) VALUES(?,?,'Keep definition',0,'every','60000','Keep user instruction',?,?,60001,'user',1,1)")
        .bind(job).bind(&owner).bind(serde_json::json!({"kind":"preset","preset_id":"0190f5fe-7c00-7a00-8000-000000000143"}).to_string()).bind(&model).execute(pool).await.unwrap();
    reset_agent_data(pool).await.unwrap();
    let (name,message,enabled,selection,binding,retained,error,next):(String,String,bool,Option<String>,Option<String>,String,String,Option<i64>)=sqlx::query_as("SELECT name,payload_message,enabled,agent_selection,agent_binding,model_selection,configuration_error,next_run_at FROM cron_jobs WHERE cron_job_id=?").bind(job).fetch_one(pool).await.unwrap();
    assert_eq!(name, "Keep definition");
    assert_eq!(message, "Keep user instruction");
    assert!(!enabled);
    assert!(selection.is_none());
    assert!(binding.is_none());
    assert_eq!(retained, model);
    assert_eq!(error, "CRON_CONFIGURATION_RESELECTION_REQUIRED");
    assert_eq!(next, None);
    let second = reset_agent_data(pool).await.unwrap();
    assert_eq!(
        second.deleted_rows["cron_jobs.agent_runtime_cleared"], 0,
        "a retained non-Agent model choice is not runtime data to reset repeatedly"
    );
}
