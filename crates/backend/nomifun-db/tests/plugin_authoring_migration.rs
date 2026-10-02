use nomifun_db::sqlx::{self, Row};
use serde_json::{Value, json};
use uuid::Uuid;

#[tokio::test]
async fn creator_history_is_preserved_as_immutable_data_and_worker_state_is_retired() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect("sqlite::memory:").await.unwrap();
    for migration in [
        include_str!("../migrations/001_canonical_baseline.sql"),
        include_str!("../migrations/002_model_compaction_threshold.sql"),
        include_str!("../migrations/003_agent_session_reasoning_effort.sql"),
        include_str!("../migrations/004_extended_agent_session_reasoning_effort.sql"),
        include_str!("../migrations/005_native_execution_checkpoints.sql"),
        include_str!("../migrations/006_native_execution_leases.sql"),
        include_str!("../migrations/007_native_pause_resume.sql"),
        include_str!("../migrations/008_plugin_draft_conversation_source.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    let owner = Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO users (user_id,username,password_hash,created_at,updated_at) VALUES (?,'migration-owner','fixture',1,1)")
        .bind(&owner).execute(&pool).await.unwrap();
    let history = json!([
        {"role":"user","content":"创建可以保存中文待办的小程序。","created_at_ms":101},
        {"role":"assistant","content":"此前创建器的回复，仅作历史资料。","created_at_ms":102}
    ]);
    let files = tempfile::tempdir().unwrap();
    let original_source = "<!doctype html><title>用户的原始草稿</title>";
    std::fs::write(files.path().join("index.html"), original_source).unwrap();
    let mut drafts = Vec::new();
    for status in ["ready", "generating", "failed"] {
        let id = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO plugin_drafts (draft_id,owner_user_id,revision,name,workspace_path,messages_json,status,last_error,created_at_ms,updated_at_ms) VALUES (?,?,7,'Original draft',?,?,?,'ORIGINAL_ERROR',123,456)")
            .bind(&id).bind(&owner).bind(files.path().to_str().unwrap()).bind(history.to_string())
            .bind(status).execute(&pool).await.unwrap();
        drafts.push((id, status));
    }
    sqlx::raw_sql(include_str!("../migrations/009_plugin_authoring_history.sql"))
        .execute(&pool).await.unwrap();
    for (id, old_status) in drafts {
        let row = sqlx::query("SELECT * FROM plugin_drafts WHERE draft_id=?")
            .bind(&id).fetch_one(&pool).await.unwrap();
        assert_eq!(row.get::<String, _>("owner_user_id"), owner);
        assert_eq!(row.get::<String, _>("workspace_path"), files.path().to_str().unwrap());
        assert_eq!(row.get::<i64, _>("created_at_ms"), 123);
        assert_eq!(row.get::<i64, _>("updated_at_ms"), 456);
        assert!(row.get::<Option<String>, _>("source_conversation_id").is_none(), "migration must not invent a conversation");
        assert!(row.get::<Option<String>, _>("source_message_id").is_none(), "migration must not invent a user message");
        let imported: Value = serde_json::from_str(&row.get::<String, _>("imported_context_json")).unwrap();
        assert_eq!(imported["messages"], history);
        assert_eq!(imported["source"], "legacy_plugin_creator");
        assert_eq!(imported["data_only"], true);
        assert_eq!(imported["legacy_status"], old_status);
        assert_eq!(row.get::<String, _>("verification_json"), "{}");
        assert_eq!(row.get::<String, _>("status"), if old_status == "generating" { "failed" } else { old_status });
        assert_eq!(row.get::<i64, _>("revision"), if old_status == "generating" { 8 } else { 7 });
        assert_eq!(row.get::<String, _>("last_error"), if old_status == "generating" { "PLUGIN_GENERATION_INTERRUPTED" } else { "ORIGINAL_ERROR" });
        assert!(sqlx::query("UPDATE plugin_drafts SET imported_context_json='{}' WHERE draft_id=?")
            .bind(&id).execute(&pool).await.is_err(), "imported history is immutable");
        assert!(sqlx::query("UPDATE plugin_drafts SET status='generating' WHERE draft_id=?")
            .bind(&id).execute(&pool).await.is_err(), "there is no independent generator state");
        sqlx::query("UPDATE plugin_drafts SET revision=revision+1 WHERE draft_id=?")
            .bind(&id).execute(&pool).await.unwrap();
    }
    let old_columns: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('plugin_drafts') WHERE name='messages_json'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(old_columns, 0);
    assert_eq!(std::fs::read_to_string(files.path().join("index.html")).unwrap(), original_source);
    let violations = sqlx::query("PRAGMA foreign_key_check").fetch_all(&pool).await.unwrap();
    assert!(violations.is_empty());
}
