use nomifun_db::{installation_owner_id, init_database_memory, models::ConversationRow, models::MessageRow};
use nomifun_common::ConversationId;
use sqlx::Row;

// Helper: insert a secondary test user and return their id.
async fn insert_test_user(pool: &sqlx::SqlitePool) -> String {
    let id = "user_0190f5fe-7c00-7a00-8abc-012345678910";
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ($1, 'testuser', 'hash', 1000, 1000)",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    id.to_string()
}

async fn insert_test_provider(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO providers (\
            id, platform, name, base_url, api_key_encrypted, models, enabled, \
            capabilities, created_at, updated_at\
         ) VALUES (?, 'openai', ?, 'https://example.invalid', \
                   'encrypted', '[]', 1, '[]', 1000, 1000)",
    )
    .bind(id)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

// Helper: insert a legal model-only Conversation for a secondary user and
// return its id. Tests that need host-capable columns resolve the installation
// owner explicitly instead.
async fn insert_test_conversation(pool: &sqlx::SqlitePool, user_id: &str) -> String {
    let id = ConversationId::new().into_string();
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, delegation_policy, created_at, updated_at) \
         VALUES ($1, $2, 'Test Chat', 'nomi', '{\"workspace\":\"/tmp\"}', \
                 'pending', 'disabled', 1000, 1000)",
    )
    .bind(&id)
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
    id
}

// -- Migration creates tables --

#[tokio::test]
async fn migration_creates_conversations_table() {
    let db = init_database_memory().await.unwrap();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(count.0, 0, "conversations table should exist and be empty");
}

#[tokio::test]
async fn migration_creates_messages_table() {
    let db = init_database_memory().await.unwrap();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(count.0, 0, "messages table should exist and be empty");
}

// -- Conversations table: column acceptance --

#[tokio::test]
async fn conversations_accepts_all_columns() {
    let db = init_database_memory().await.unwrap();
    insert_test_provider(db.pool(), "prov_0190f5fe-7c00-7a00-8abc-012345678911").await;
    let user_id = installation_owner_id(db.pool()).await.unwrap();

    let conversation_id = ConversationId::new().into_string();
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, model, status, source, channel_chat_id, \
          pinned, pinned_at, created_at, updated_at) \
         VALUES ($1, $2, 'Full Chat', 'acp', '{\"backend\":\"claude\"}', \
                 '{\"provider_id\":\"prov_0190f5fe-7c00-7a00-8abc-012345678911\",\"model\":\"claude-sonnet\"}', \
                 'running', 'telegram', 'user:123', 1, 1700000000000, 1000, 2000)",
    )
    .bind(&conversation_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT * FROM conversations WHERE id = ?")
        .bind(&conversation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.get::<String, _>("name"), "Full Chat");
    assert_eq!(row.get::<String, _>("type"), "acp");
    assert_eq!(row.get::<String, _>("status"), "running");
    assert_eq!(row.get::<String, _>("source"), "telegram");
    assert_eq!(row.get::<String, _>("channel_chat_id"), "user:123");
    assert_eq!(row.get::<i32, _>("pinned"), 1);
    assert_eq!(row.get::<i64, _>("pinned_at"), 1700000000000);
}

#[tokio::test]
async fn conversations_reject_legacy_model_identity_shapes() {
    let db = init_database_memory().await.unwrap();
    let provider_id = "prov_0190f5fe-7c00-7a00-8abc-012345678912";
    insert_test_provider(db.pool(), provider_id).await;
    let user_id = installation_owner_id(db.pool()).await.unwrap();

    for model in [
        format!(r#"{{"providerId":"{provider_id}","model":"m1"}}"#),
        format!(r#"{{"id":"{provider_id}","model":"m1"}}"#),
        format!(r#"{{"provider_id":"{provider_id}","useModel":"m1","model":"m1"}}"#),
    ] {
        let error = sqlx::query(
            "INSERT INTO conversations \
             (id, user_id, name, type, extra, model, status, delegation_policy, created_at, updated_at) \
             VALUES (?, ?, 'legacy model', 'nomi', '{}', ?, 'pending', 'disabled', 1000, 1000)",
        )
        .bind(ConversationId::new().into_string())
        .bind(&user_id)
        .bind(model)
        .execute(db.pool())
        .await
        .unwrap_err();
        assert!(
            error.to_string().to_ascii_lowercase().contains("conversation model"),
            "unexpected error: {error}"
        );
    }
}

// -- Conversations table: default values --

#[tokio::test]
async fn conversations_defaults() {
    let db = init_database_memory().await.unwrap();
    let user_id = installation_owner_id(db.pool()).await.unwrap();

    let conversation_id = ConversationId::new().into_string();
    sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
         VALUES ($1, $2, 'Default Chat', 'gemini', 'pending', 1000, 1000)",
    )
    .bind(&conversation_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT extra, pinned, pinned_at, model, source FROM conversations WHERE id = ?")
        .bind(&conversation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(
        row.get::<String, _>("extra"),
        "{}",
        "extra should default to empty JSON"
    );
    assert_eq!(row.get::<i32, _>("pinned"), 0, "pinned should default to 0");
    assert!(
        row.get::<Option<i64>, _>("pinned_at").is_none(),
        "pinned_at should default to NULL"
    );
    assert!(
        row.get::<Option<String>, _>("model").is_none(),
        "model should default to NULL"
    );
    assert!(
        row.get::<Option<String>, _>("source").is_none(),
        "source should default to NULL"
    );
}

// -- Conversations table: CHECK constraints --

#[tokio::test]
async fn conversations_status_check_constraint() {
    let db = init_database_memory().await.unwrap();
    let user_id = installation_owner_id(db.pool()).await.unwrap();
    let conversation_id = ConversationId::new().into_string();

    let result = sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
         VALUES ($1, $2, 'Bad', 'gemini', 'invalid_status', 1000, 1000)",
    )
    .bind(&conversation_id)
    .bind(&user_id)
    .execute(db.pool())
    .await;

    assert!(result.is_err(), "invalid status should violate CHECK constraint");
}

#[tokio::test]
async fn conversations_status_allows_valid_values() {
    let db = init_database_memory().await.unwrap();
    let user_id = installation_owner_id(db.pool()).await.unwrap();

    for status in ["pending", "running", "finished"] {
        let id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
             VALUES ($1, $2, 'Test', 'gemini', $3, 1000, 1000)",
        )
        .bind(&id)
        .bind(&user_id)
        .bind(status)
        .execute(db.pool())
        .await
        .unwrap_or_else(|e| panic!("status '{status}' should be valid: {e}"));
    }
}

// -- FK constraint: user_id --

#[tokio::test]
async fn conversations_fk_user_id() {
    let db = init_database_memory().await.unwrap();
    let conversation_id = ConversationId::new().into_string();

    let result = sqlx::query(
        "INSERT INTO conversations \
            (id, user_id, name, type, status, delegation_policy, created_at, updated_at) \
         VALUES \
            ($1, 'nonexistent_user', 'Bad FK', 'nomi', 'pending', 'disabled', 1000, 1000)",
    )
    .bind(&conversation_id)
    .execute(db.pool())
    .await;

    assert!(result.is_err(), "non-existent user_id should violate FK constraint");
}

// -- CASCADE delete: users → conversations --

#[tokio::test]
async fn cascade_delete_user_removes_conversations() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    insert_test_conversation(db.pool(), &user_id).await;

    // Verify conversation exists
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE user_id = $1")
        .bind(&user_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count.0, 1);

    // Delete user
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(&user_id)
        .execute(db.pool())
        .await
        .unwrap();

    // Conversations should be gone
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE user_id = $1")
        .bind(&user_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count.0, 0, "conversations should be cascade-deleted with user");
}

// -- Messages table: column acceptance --

#[tokio::test]
async fn messages_accepts_all_columns() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    sqlx::query(
        "INSERT INTO messages \
         (id, conversation_id, msg_id, type, content, position, status, hidden, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678921', $1, 'msg_0190f5fe-7c00-7a00-8abc-012345678922', 'text', \
                 '{\"content\":\"Hello\"}', 'right', 'finish', 0, 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT * FROM messages WHERE id = 'msg_0190f5fe-7c00-7a00-8abc-012345678921'")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.get::<String, _>("conversation_id"), conv_id);
    assert_eq!(row.get::<String, _>("msg_id"), "msg_0190f5fe-7c00-7a00-8abc-012345678922");
    assert_eq!(row.get::<String, _>("type"), "text");
    assert_eq!(row.get::<String, _>("position"), "right");
    assert_eq!(row.get::<String, _>("status"), "finish");
    assert_eq!(row.get::<i32, _>("hidden"), 0);
}

// -- Messages table: default values --

#[tokio::test]
async fn messages_defaults() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    sqlx::query(
        "INSERT INTO messages (id, conversation_id, type, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678923', $1, 'text', 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT content, hidden, msg_id, position, status FROM messages WHERE id = 'msg_0190f5fe-7c00-7a00-8abc-012345678923'")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(
        row.get::<String, _>("content"),
        "{}",
        "content should default to empty JSON"
    );
    assert_eq!(row.get::<i32, _>("hidden"), 0, "hidden should default to 0");
    assert!(row.get::<Option<String>, _>("msg_id").is_none());
    assert!(row.get::<Option<String>, _>("position").is_none());
    assert!(row.get::<Option<String>, _>("status").is_none());
}

// -- Messages table: CHECK constraints --

#[tokio::test]
async fn messages_position_check_constraint() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    let result = sqlx::query(
        "INSERT INTO messages (id, conversation_id, type, position, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678924', $1, 'text', 'invalid_pos', 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await;

    assert!(result.is_err(), "invalid position should violate CHECK constraint");
}

#[tokio::test]
async fn messages_status_check_constraint() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    let result = sqlx::query(
        "INSERT INTO messages (id, conversation_id, type, status, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678925', $1, 'text', 'invalid_status', 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await;

    assert!(result.is_err(), "invalid status should violate CHECK constraint");
}

#[tokio::test]
async fn messages_allows_valid_positions() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    for (i, pos) in ["left", "right", "center", "pop"].iter().enumerate() {
        let id = format!("msg_p{}", i);
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, type, position, created_at) \
             VALUES ($1, $2, 'text', $3, 1000)",
        )
        .bind(&id)
        .bind(&conv_id)
        .bind(pos)
        .execute(db.pool())
        .await
        .unwrap_or_else(|e| panic!("position '{pos}' should be valid: {e}"));
    }
}

#[tokio::test]
async fn messages_allows_valid_statuses() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    for (i, status) in ["finish", "pending", "error", "work"].iter().enumerate() {
        let id = format!("msg_s{}", i);
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, type, status, created_at) \
             VALUES ($1, $2, 'text', $3, 1000)",
        )
        .bind(&id)
        .bind(&conv_id)
        .bind(status)
        .execute(db.pool())
        .await
        .unwrap_or_else(|e| panic!("status '{status}' should be valid: {e}"));
    }
}

// -- FK constraint: conversation_id --

#[tokio::test]
async fn messages_fk_conversation_id() {
    let db = init_database_memory().await.unwrap();

    let result = sqlx::query(
        "INSERT INTO messages (id, conversation_id, type, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678926', 'conv_0190f5fe-7c00-7a00-8abc-012345678999', 'text', 1000)",
    )
    .execute(db.pool())
    .await;

    assert!(
        result.is_err(),
        "non-existent conversation_id should violate FK constraint"
    );
}

// -- CASCADE delete: conversations → messages --

#[tokio::test]
async fn cascade_delete_conversation_removes_messages() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    // Insert messages
    for i in 0..3 {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, type, content, created_at) \
             VALUES ($1, $2, 'text', '{\"content\":\"msg\"}', 1000)",
        )
        .bind(format!("msg_{}", i))
        .bind(&conv_id)
        .execute(db.pool())
        .await
        .unwrap();
    }

    // Verify messages exist
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE conversation_id = $1")
        .bind(&conv_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count.0, 3);

    // Delete conversation
    sqlx::query("DELETE FROM conversations WHERE id = $1")
        .bind(&conv_id)
        .execute(db.pool())
        .await
        .unwrap();

    // Messages should be gone
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE conversation_id = $1")
        .bind(&conv_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count.0, 0, "messages should be cascade-deleted with conversation");
}

// -- Full cascade: users → conversations → messages --

#[tokio::test]
async fn cascade_delete_user_removes_conversations_and_messages() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    sqlx::query(
        "INSERT INTO messages (id, conversation_id, type, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678927', $1, 'text', 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    // Delete user — should cascade to conversations and messages
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(&user_id)
        .execute(db.pool())
        .await
        .unwrap();

    let conv_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
        .fetch_one(db.pool())
        .await
        .unwrap();
    let msg_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(conv_count.0, 0, "conversations should be cascade-deleted");
    assert_eq!(msg_count.0, 0, "messages should be cascade-deleted");
}

// -- FromRow: ConversationRow --

#[tokio::test]
async fn conversation_row_from_row() {
    let db = init_database_memory().await.unwrap();
    insert_test_provider(db.pool(), "prov_0190f5fe-7c00-7a00-8abc-012345678911").await;
    let user_id = installation_owner_id(db.pool()).await.unwrap();
    let conversation_id = ConversationId::new().into_string();

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, model, status, source, channel_chat_id, \
          pinned, pinned_at, created_at, updated_at) \
         VALUES ($1, $2, 'FromRow Test', 'gemini', '{\"workspace\":\"/home\"}', \
                 '{\"provider_id\":\"prov_0190f5fe-7c00-7a00-8abc-012345678911\",\"model\":\"m1\"}', \
                 'finished', 'nomifun', 'group:42', 1, 1700000000000, 1000, 2000)",
    )
    .bind(&conversation_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: ConversationRow = sqlx::query_as("SELECT * FROM conversations WHERE id = ?")
        .bind(&conversation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.id, conversation_id);
    assert_eq!(row.user_id, user_id);
    assert_eq!(row.name, "FromRow Test");
    assert_eq!(row.r#type, "gemini");
    assert_eq!(row.extra, "{\"workspace\":\"/home\"}");
    assert_eq!(row.model.as_deref(), Some("{\"provider_id\":\"prov_0190f5fe-7c00-7a00-8abc-012345678911\",\"model\":\"m1\"}"));
    assert_eq!(row.status.as_deref(), Some("finished"));
    assert_eq!(row.source.as_deref(), Some("nomifun"));
    assert_eq!(row.channel_chat_id.as_deref(), Some("group:42"));
    assert!(row.pinned);
    assert_eq!(row.pinned_at, Some(1700000000000));
    assert_eq!(row.created_at, 1000);
    assert_eq!(row.updated_at, 2000);
}

#[tokio::test]
async fn conversation_row_nullable_fields() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conversation_id = ConversationId::new().into_string();

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, delegation_policy, created_at, updated_at) \
         VALUES \
            ($1, $2, 'Nullable Test', 'nomi', '{}', 'pending', 'disabled', 1000, 1000)",
    )
    .bind(&conversation_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: ConversationRow = sqlx::query_as("SELECT * FROM conversations WHERE id = ?")
        .bind(&conversation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert!(row.model.is_none());
    assert!(row.source.is_none());
    assert!(row.channel_chat_id.is_none());
    assert!(!row.pinned);
    assert!(row.pinned_at.is_none());
}

// -- FromRow: MessageRow --

#[tokio::test]
async fn message_row_from_row() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    sqlx::query(
        "INSERT INTO messages \
         (id, conversation_id, msg_id, type, content, position, status, hidden, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678928', $1, 'msg_0190f5fe-7c00-7a00-8abc-012345678929', 'text', '{\"content\":\"Hi\"}', \
                 'right', 'finish', 1, 1500)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: MessageRow = sqlx::query_as("SELECT * FROM messages WHERE id = 'msg_0190f5fe-7c00-7a00-8abc-012345678928'")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.id, "msg_0190f5fe-7c00-7a00-8abc-012345678928");
    assert_eq!(row.conversation_id, conv_id);
    assert_eq!(row.msg_id.as_deref(), Some("msg_0190f5fe-7c00-7a00-8abc-012345678929"));
    assert_eq!(row.r#type, "text");
    assert_eq!(row.content, "{\"content\":\"Hi\"}");
    assert_eq!(row.position.as_deref(), Some("right"));
    assert_eq!(row.status.as_deref(), Some("finish"));
    assert!(row.hidden);
    assert_eq!(row.created_at, 1500);
}

#[tokio::test]
async fn message_row_nullable_fields() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;
    let conv_id = insert_test_conversation(db.pool(), &user_id).await;

    sqlx::query(
        "INSERT INTO messages (id, conversation_id, type, created_at) \
         VALUES ('msg_0190f5fe-7c00-7a00-8abc-012345678930', $1, 'tips', 2000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: MessageRow = sqlx::query_as("SELECT * FROM messages WHERE id = 'msg_0190f5fe-7c00-7a00-8abc-012345678930'")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert!(row.msg_id.is_none());
    assert!(row.position.is_none());
    assert!(row.status.is_none());
    assert!(!row.hidden);
    assert_eq!(row.content, "{}");
}

// -- Index existence --

#[tokio::test]
async fn conversation_indexes_exist() {
    let db = init_database_memory().await.unwrap();

    let indexes: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master \
         WHERE type = 'index' AND tbl_name = 'conversations' AND name LIKE 'idx_%'",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();

    let names: Vec<&str> = indexes.iter().map(|r| r.0.as_str()).collect();
    assert!(names.contains(&"idx_conversations_user_id"));
    assert!(names.contains(&"idx_conversations_updated_at"));
    assert!(names.contains(&"idx_conversations_type"));
    assert!(names.contains(&"idx_conversations_user_updated"));
    assert!(names.contains(&"idx_conversations_source"));
    assert!(names.contains(&"idx_conversations_source_updated"));
}

#[tokio::test]
async fn message_indexes_exist() {
    let db = init_database_memory().await.unwrap();

    let indexes: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master \
         WHERE type = 'index' AND tbl_name = 'messages' AND name LIKE 'idx_%'",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();

    let names: Vec<&str> = indexes.iter().map(|r| r.0.as_str()).collect();
    assert!(names.contains(&"idx_messages_conversation_id"));
    assert!(names.contains(&"idx_messages_created_at"));
    assert!(names.contains(&"idx_messages_type"));
    assert!(names.contains(&"idx_messages_msg_id"));
    assert!(names.contains(&"idx_messages_conv_created"));
}
