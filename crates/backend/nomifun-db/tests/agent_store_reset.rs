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

#[tokio::test]
async fn reset_retires_session_references_and_preserves_business_state_and_configuration() {
    use nomifun_common::{ConversationId, CreativeStudioProjectId, KnowledgeBaseId, KnowledgeBindingId, MessageId, ProviderId, RequirementId, TerminalId};
    use serde_json::{Value, json};

    let db = init_database_memory().await.unwrap();
    let pool = db.pool();
    let owner = nomifun_db::installation_owner_id(pool).await.unwrap();
    let session = ConversationId::new().into_string();
    let terminal = TerminalId::new().into_string();
    let base = KnowledgeBaseId::new().into_string();
    let session_binding = KnowledgeBindingId::new().into_string();
    let workpath_binding = KnowledgeBindingId::new().into_string();
    let canvas = CreativeStudioProjectId::new().into_string();
    let provider = ProviderId::new().into_string();
    let chat = ConversationId::new().into_string();
    let pending_chat = ConversationId::new().into_string();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,'{}','live','Reset source',0,0,'{}',1,1)")
        .bind(&session).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO terminal_sessions(terminal_id,name,cwd,command,user_id,created_at,updated_at) VALUES(?,'Retained terminal','/tmp','shell',?,1,1)")
        .bind(&terminal).bind(&owner).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO providers(provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,created_at,updated_at) VALUES(?,'openai','Retained canvas model','https://example.invalid','bearer','retained credential',1,1)")
        .bind(&provider).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO knowledge_bases(knowledge_base_id,name,root_path,created_at,updated_at) VALUES(?,'Retained knowledge','/tmp/retained-knowledge',1,1)")
        .bind(&base).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO knowledge_bindings(knowledge_binding_id,target_kind,target_conversation_id,enabled,updated_at) VALUES(?,'conversation',?,1,1)")
        .bind(&session_binding).bind(&session).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO knowledge_bindings(knowledge_binding_id,target_kind,target_workpath,enabled,updated_at) VALUES(?,'workpath','/tmp/retained-workpath',1,1)")
        .bind(&workpath_binding).execute(pool).await.unwrap();
    for binding in [&session_binding, &workpath_binding] {
        sqlx::query("INSERT INTO knowledge_binding_bases(knowledge_binding_id,knowledge_base_id,position) VALUES(?,?,0)")
            .bind(binding).bind(&base).execute(pool).await.unwrap();
    }
    for (key, value) in [
        (format!("agent_session.idmm.{session}"), json!({"session_id":session,"config":{"enabled":true}}).to_string()),
        ("guid.defaultAgentSelection".into(), "retired selection".into()),
        ("retained-user-setting".into(), "exact retained value".into()),
    ] {
        sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES(?,?,1)")
            .bind(key).bind(value).execute(pool).await.unwrap();
    }

    let mut requirements = Vec::new();
    for (index, status) in ["pending", "done", "failed", "cancelled", "needs_review", "in_progress"]
        .into_iter().enumerate()
    {
        let id = RequirementId::new().into_string();
        let note = format!("original {status} note");
        sqlx::query("INSERT INTO requirements(requirement_id,display_no,title,tag,completion_note,created_at,updated_at) VALUES(?,?,'Retained business task','reset',?,1,1)")
            .bind(&id).bind(index as i64 + 1).bind(&note).execute(pool).await.unwrap();
        if status != "pending" {
            sqlx::query("UPDATE requirements SET status='in_progress',owner_conversation_id=?,active_turn_started_at=10,lease_expires_at=100,started_at=10,attempt_count=1,claim_generation=1,claim_token=? WHERE requirement_id=?")
                .bind(&session).bind("a".repeat(64)).bind(&id).execute(pool).await.unwrap();
            if status != "in_progress" {
                sqlx::query("UPDATE requirements SET status=? WHERE requirement_id=?")
                    .bind(status).bind(&id).execute(pool).await.unwrap();
            }
        }
        requirements.push((id, status, note));
    }
    let terminal_requirement = RequirementId::new().into_string();
    sqlx::query("INSERT INTO requirements(requirement_id,display_no,title,tag,created_at,updated_at) VALUES(?,7,'Retained terminal task','terminal-reset',1,1)")
        .bind(&terminal_requirement).execute(pool).await.unwrap();
    sqlx::query("UPDATE requirements SET status='in_progress',owner_terminal_id=?,active_turn_started_at=10,lease_expires_at=100,started_at=10,attempt_count=1,claim_generation=1,claim_token=? WHERE requirement_id=?")
        .bind(&terminal).bind("b".repeat(64)).bind(&terminal_requirement).execute(pool).await.unwrap();

    let document = json!({
        "schema":"nomifun.creative-studio/v1", "projectId":canvas,
        "viewport":{"x":12,"y":34,"zoom":1.5}, "background":"dots",
        "nodes":[], "connections":[], "panels":{
            "left":{"open":true,"width":280,"activeView":"canvas"},
            "right":{"open":true,"width":390,"activeView":"assistant"},
            "bottom":{"open":false,"height":240,"activeView":"history"}
        },
        "chatSessions":[
            {"id":chat,"title":"Retain history title","model":{"providerId":provider,"model":"retained-model"},
             "messageIds":[MessageId::new().into_string(),MessageId::new().into_string()],
             "pendingTurn":null,"createdAt":1,"updatedAt":2},
            {"id":pending_chat,"title":"Retain pending title","model":{"providerId":provider,"model":"pending-model"},
             "messageIds":[],"pendingTurn":{"idempotencyKey":ConversationId::new().into_string(),"prompt":"retired pending input","createdAt":2},
             "createdAt":1,"updatedAt":2}
        ],
        "activeChatId":pending_chat,"pendingTaskIds":[ConversationId::new().into_string()]
    });
    sqlx::query("INSERT INTO creative_studio_projects(project_id,title,document_json,revision,created_at,updated_at) VALUES(?,'Retained canvas',?,2,1,2)")
        .bind(&canvas).bind(document.to_string()).execute(pool).await.unwrap();

    let report = reset_agent_data(pool).await.unwrap();
    assert_eq!(report.deleted_rows["knowledge_bindings.session_rows"], 1);
    assert_eq!(report.deleted_rows["client_preferences.agent_session_rows"], 1);
    assert_eq!(report.deleted_rows["creative_studio_projects.agent_projections_cleared"], 1);
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT value FROM client_preferences WHERE key='retained-user-setting'").fetch_one(pool).await.unwrap(), "exact retained value");
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM knowledge_bindings").fetch_one(pool).await.unwrap(), 1);
    let retained_binding: String = sqlx::query_scalar("SELECT knowledge_binding_id FROM knowledge_binding_bases").fetch_one(pool).await.unwrap();
    assert_eq!(retained_binding, workpath_binding);
    for (id, previous_status, note) in requirements {
        let row: (String, Option<String>, Option<String>, Option<String>, i64) = sqlx::query_as(
            "SELECT status,completion_note,owner_conversation_id,claim_token,attempt_count FROM requirements WHERE requirement_id=?"
        ).bind(id).fetch_one(pool).await.unwrap();
        assert_eq!(row.0, if previous_status == "in_progress" { "needs_review" } else { previous_status });
        assert_eq!(row.1, Some(note));
        assert!(row.2.is_none());
        assert!(row.3.is_none());
        assert_eq!(row.4, i64::from(previous_status != "pending"));
    }
    let terminal_state: (String, String, String) = sqlx::query_as("SELECT status,owner_terminal_id,claim_token FROM requirements WHERE requirement_id=?")
        .bind(terminal_requirement).fetch_one(pool).await.unwrap();
    assert_eq!(terminal_state, ("in_progress".into(), terminal, "b".repeat(64)));
    let (stored, revision): (String, i64) = sqlx::query_as("SELECT document_json,revision FROM creative_studio_projects WHERE project_id=?")
        .bind(&canvas).fetch_one(pool).await.unwrap();
    let mut expected = document;
    for chat in expected["chatSessions"].as_array_mut().unwrap() {
        chat["messageIds"] = json!([]);
        chat["pendingTurn"] = Value::Null;
    }
    expected["pendingTaskIds"] = json!([]);
    assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), expected);
    assert_eq!(revision, 3);
    nomifun_db::validate_id_data_contract(pool).await.unwrap();
    let second = reset_agent_data(pool).await.unwrap();
    assert_eq!(second.deleted_rows["creative_studio_projects.agent_projections_cleared"], 0);
    assert_eq!(second.deleted_rows["knowledge_bindings.session_rows"], 0);
    assert_eq!(second.deleted_rows["client_preferences.agent_session_rows"], 0);
}
