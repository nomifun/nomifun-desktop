use nomifun_db::{init_database_memory, models::ConversationRow, models::MessageRow};
use sqlx::Row;

// Helper: insert a test user and return their id.
async fn insert_test_user(pool: &sqlx::SqlitePool) -> String {
    let id = "test_user_1";
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

// Helper: insert a test conversation and return its id. The explicit integer
// id is a valid AUTOINCREMENT rowid.
async fn insert_test_conversation(pool: &sqlx::SqlitePool, user_id: &str) -> i64 {
    let id: i64 = 1;
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, created_at, updated_at) \
         VALUES ($1, $2, 'Test Chat', 'gemini', '{\"workspace\":\"/tmp\"}', 'pending', 1000, 1000)",
    )
    .bind(id)
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
    let user_id = insert_test_user(db.pool()).await;

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, model, status, source, channel_chat_id, \
          pinned, pinned_at, created_at, updated_at) \
         VALUES ($1, $2, 'Full Chat', 'acp', '{\"backend\":\"claude\"}', \
                 '{\"providerId\":\"p1\",\"model\":\"claude-sonnet\"}', \
                 'running', 'telegram', 'user:123', 1, 1700000000000, 1000, 2000)",
    )
    .bind(10_i64)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT * FROM conversations WHERE id = 10")
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

// -- Conversations table: default values --

#[tokio::test]
async fn conversations_defaults() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;

    sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
         VALUES (11, $1, 'Default Chat', 'gemini', 'pending', 1000, 1000)",
    )
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT extra, pinned, pinned_at, model, source FROM conversations WHERE id = 11")
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
    let user_id = insert_test_user(db.pool()).await;

    let result = sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
         VALUES (12, $1, 'Bad', 'gemini', 'invalid_status', 1000, 1000)",
    )
    .bind(&user_id)
    .execute(db.pool())
    .await;

    assert!(result.is_err(), "invalid status should violate CHECK constraint");
}

#[tokio::test]
async fn conversations_status_allows_valid_values() {
    let db = init_database_memory().await.unwrap();
    let user_id = insert_test_user(db.pool()).await;

    for (i, status) in ["pending", "running", "finished"].iter().enumerate() {
        let id = 20_i64 + i as i64;
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
             VALUES ($1, $2, 'Test', 'gemini', $3, 1000, 1000)",
        )
        .bind(id)
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

    let result = sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
         VALUES (30, 'nonexistent_user', 'Bad FK', 'gemini', 'pending', 1000, 1000)",
    )
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
         VALUES ('msg_1', $1, 'client_msg_1', 'text', \
                 '{\"content\":\"Hello\"}', 'right', 'finish', 0, 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT * FROM messages WHERE id = 'msg_1'")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.get::<i64, _>("conversation_id"), conv_id);
    assert_eq!(row.get::<String, _>("msg_id"), "client_msg_1");
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
         VALUES ('msg_def', $1, 'text', 1000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row = sqlx::query("SELECT content, hidden, msg_id, position, status FROM messages WHERE id = 'msg_def'")
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
         VALUES ('msg_bad_pos', $1, 'text', 'invalid_pos', 1000)",
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
         VALUES ('msg_bad_st', $1, 'text', 'invalid_status', 1000)",
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
         VALUES ('msg_fk', 999999, 'text', 1000)",
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
         VALUES ('msg_cascade', $1, 'text', 1000)",
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
    let user_id = insert_test_user(db.pool()).await;

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, model, status, source, channel_chat_id, \
          pinned, pinned_at, created_at, updated_at) \
         VALUES (40, $1, 'FromRow Test', 'gemini', '{\"workspace\":\"/home\"}', \
                 '{\"providerId\":\"p1\",\"model\":\"m1\"}', \
                 'finished', 'nomifun', 'group:42', 1, 1700000000000, 1000, 2000)",
    )
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: ConversationRow = sqlx::query_as("SELECT * FROM conversations WHERE id = 40")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.id, 40);
    assert_eq!(row.user_id, user_id);
    assert_eq!(row.name, "FromRow Test");
    assert_eq!(row.r#type, "gemini");
    assert_eq!(row.extra, "{\"workspace\":\"/home\"}");
    assert_eq!(row.model.as_deref(), Some("{\"providerId\":\"p1\",\"model\":\"m1\"}"));
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

    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, created_at, updated_at) \
         VALUES (41, $1, 'Nullable Test', 'remote', '{}', 'pending', 1000, 1000)",
    )
    .bind(&user_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: ConversationRow = sqlx::query_as("SELECT * FROM conversations WHERE id = 41")
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
         VALUES ('msg_fr', $1, 'client_42', 'text', '{\"content\":\"Hi\"}', \
                 'right', 'finish', 1, 1500)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: MessageRow = sqlx::query_as("SELECT * FROM messages WHERE id = 'msg_fr'")
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.id, "msg_fr");
    assert_eq!(row.conversation_id, conv_id);
    assert_eq!(row.msg_id.as_deref(), Some("client_42"));
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
         VALUES ('msg_null', $1, 'tips', 2000)",
    )
    .bind(&conv_id)
    .execute(db.pool())
    .await
    .unwrap();

    let row: MessageRow = sqlx::query_as("SELECT * FROM messages WHERE id = 'msg_null'")
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
