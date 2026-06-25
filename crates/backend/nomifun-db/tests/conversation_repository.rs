use nomifun_db::{
    ConversationFilters, ConversationRowUpdate, IConversationRepository, MessageRowUpdate, SortOrder,
    SqliteConversationRepository, init_database_memory, models::ConversationRow, models::MessageRow,
};

const USER_ID: &str = "system_default_user";

async fn setup() -> (SqliteConversationRepository, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let repo = SqliteConversationRepository::new(db.pool().clone());
    (repo, db)
}

fn make_conversation(suffix: &str) -> ConversationRow {
    let now = nomifun_common::now_ms();
    ConversationRow {
        // id is allocated by SQLite on create(); the value here is ignored.
        id: 0,
        user_id: USER_ID.to_string(),
        name: format!("Conversation {suffix}"),
        r#type: "gemini".to_string(),
        extra: r#"{"workspace":"/home/user/project"}"#.to_string(),
        model: Some(r#"{"providerId":"prov_1","model":"claude-sonnet-4-20250514"}"#.to_string()),
        status: Some("pending".to_string()),
        source: Some("nomifun".to_string()),
        channel_chat_id: None,
        pinned: false,
        pinned_at: None,
        cron_job_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn make_message(conv_id: i64, content: &str) -> MessageRow {
    let now = nomifun_common::now_ms();
    MessageRow {
        id: nomifun_common::generate_prefixed_id("msg"),
        conversation_id: conv_id,
        msg_id: Some(nomifun_common::generate_prefixed_id("cmsg")),
        r#type: "text".to_string(),
        content: format!(r#"{{"content":"{content}"}}"#),
        position: Some("right".to_string()),
        status: Some("finish".to_string()),
        hidden: false,
        created_at: now,
    }
}

fn make_artifact(conv_id: i64) -> nomifun_db::ConversationArtifactRow {
    nomifun_db::ConversationArtifactRow {
        // id is ignored on upsert (INTEGER PK AUTOINCREMENT); any value works.
        id: 0,
        conversation_id: conv_id,
        cron_job_id: Some("cron_1".to_string()),
        kind: "skill_suggest".to_string(),
        status: "pending".to_string(),
        payload: serde_json::json!({
            "cron_job_id": "cron_1",
            "name": "daily-report",
            "description": "Daily report",
            "skillContent": "---\nname: daily-report\n---\nUse it."
        })
        .to_string(),
        created_at: 1000,
        updated_at: 1000,
    }
}

/// Insert a minimal `cron_jobs` row so the artifact FK
/// (`conversation_artifacts.cron_job_id → cron_jobs(id)`) is satisfiable.
/// conversation_id is left NULL to avoid an extra FK dependency.
async fn seed_cron_job(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO cron_jobs \
            (id, name, schedule_kind, schedule_value, payload_message, agent_type, created_by, created_at, updated_at) \
         VALUES (?, 'Job', 'every', '60000', 'msg', 'acp', 'user', 0, 0)",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

// ── Conversation CRUD ───────────────────────────────────────────────

#[tokio::test]
async fn create_get_update_delete_lifecycle() {
    let (repo, _db) = setup().await;

    // Create
    let mut conv = make_conversation("lifecycle");
    conv.id = repo.create(&conv).await.unwrap();

    // Get
    let found = repo.get(conv.id).await.unwrap().unwrap();
    assert_eq!(found.name, "Conversation lifecycle");
    assert_eq!(found.status.as_deref(), Some("pending"));

    // Update
    let now = nomifun_common::now_ms();
    repo.update(
        conv.id,
        &ConversationRowUpdate {
            name: Some("Updated Name".to_string()),
            status: Some("running".to_string()),
            updated_at: Some(now),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let updated = repo.get(conv.id).await.unwrap().unwrap();
    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.status.as_deref(), Some("running"));

    // Delete
    repo.delete(conv.id).await.unwrap();
    assert!(repo.get(conv.id).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_conversation_cascades_messages() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("cascade");
    conv.id = repo.create(&conv).await.unwrap();

    // Insert messages
    for i in 0..3 {
        let msg = make_message(conv.id, &format!("msg {i}"));
        repo.insert_message(&msg).await.unwrap();
    }

    // Verify messages exist
    let msgs = repo.get_messages(conv.id, 1, 50, SortOrder::Desc).await.unwrap();
    assert_eq!(msgs.total, 3);

    // Delete conversation → messages cascade
    repo.delete(conv.id).await.unwrap();

    let msgs = repo.get_messages(conv.id, 1, 50, SortOrder::Desc).await.unwrap();
    assert_eq!(msgs.total, 0);
}

// ── Cursor pagination ───────────────────────────────────────────────

#[tokio::test]
async fn cursor_pagination_walks_through_all_items() {
    let (repo, _db) = setup().await;

    // Create 7 conversations with distinct updated_at
    for i in 0..7 {
        let mut c = make_conversation(&format!("{i}"));
        c.updated_at = (i + 1) as i64 * 1000;
        repo.create(&c).await.unwrap();
    }

    // Page 1: no cursor, limit 3
    let p1 = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                limit: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p1.items.len(), 3);
    assert!(p1.has_more);
    assert_eq!(p1.total, 7);

    // Page 2
    let cursor = p1.items.last().unwrap().id.clone();
    let p2 = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                cursor: Some(cursor),
                limit: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p2.items.len(), 3);
    assert!(p2.has_more);

    // Page 3
    let cursor = p2.items.last().unwrap().id.clone();
    let p3 = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                cursor: Some(cursor),
                limit: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p3.items.len(), 1);
    assert!(!p3.has_more);

    // All 7 items collected, no duplicates
    let mut all_ids: Vec<_> = p1
        .items
        .iter()
        .chain(p2.items.iter())
        .chain(p3.items.iter())
        .map(|c| c.id.clone())
        .collect();
    all_ids.sort();
    all_ids.dedup();
    assert_eq!(all_ids.len(), 7);
}

// ── Filter combinations ─────────────────────────────────────────────

#[tokio::test]
async fn filter_by_source_and_pinned_combined() {
    let (repo, _db) = setup().await;

    let mut c1 = make_conversation("nomifun-pinned");
    c1.source = Some("nomifun".to_string());
    c1.pinned = true;
    c1.pinned_at = Some(nomifun_common::now_ms());
    c1.id = repo.create(&c1).await.unwrap();

    let mut c2 = make_conversation("telegram-pinned");
    c2.source = Some("telegram".to_string());
    c2.pinned = true;
    c2.pinned_at = Some(nomifun_common::now_ms());
    repo.create(&c2).await.unwrap();

    let mut c3 = make_conversation("nomifun-unpinned");
    c3.source = Some("nomifun".to_string());
    c3.pinned = false;
    repo.create(&c3).await.unwrap();

    // Filter: source=nomifun AND pinned=true
    let result = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                source: Some("nomifun".to_string()),
                pinned: Some(true),
                limit: 20,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, c1.id);
}

#[tokio::test]
async fn filter_by_cron_job_id() {
    let (repo, db) = setup().await;
    seed_cron_job(db.pool(), "cron_123").await;
    seed_cron_job(db.pool(), "cron_456").await;

    let mut c1 = make_conversation("cron-a");
    c1.cron_job_id = Some("cron_123".to_string());
    c1.id = repo.create(&c1).await.unwrap();

    let mut c2 = make_conversation("cron-b");
    c2.cron_job_id = Some("cron_456".to_string());
    repo.create(&c2).await.unwrap();

    let c3 = make_conversation("no-cron"); // cron_job_id is None
    repo.create(&c3).await.unwrap();

    let result = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                cron_job_id: Some("cron_123".to_string()),
                limit: 20,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, c1.id);
}

// ── Extended queries ────────────────────────────────────────────────

#[tokio::test]
async fn find_by_source_and_chat_integration() {
    let (repo, _db) = setup().await;

    let mut c = make_conversation("telegram");
    c.source = Some("telegram".to_string());
    c.channel_chat_id = Some("group:789".to_string());
    c.r#type = "acp".to_string();
    c.id = repo.create(&c).await.unwrap();

    let found = repo
        .find_by_source_and_chat(USER_ID, "telegram", "group:789", "acp")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, c.id);
}

#[tokio::test]
async fn list_by_cron_job_returns_matching() {
    let (repo, db) = setup().await;
    seed_cron_job(db.pool(), "job_x").await;
    seed_cron_job(db.pool(), "job_y").await;

    let mut c1 = make_conversation("cron1");
    c1.cron_job_id = Some("job_x".to_string());
    repo.create(&c1).await.unwrap();

    let mut c2 = make_conversation("cron2");
    c2.cron_job_id = Some("job_x".to_string());
    repo.create(&c2).await.unwrap();

    let mut c3 = make_conversation("cron3");
    c3.cron_job_id = Some("job_y".to_string());
    repo.create(&c3).await.unwrap();

    let result = repo.list_by_cron_job(USER_ID, "job_x").await.unwrap();
    assert_eq!(result.len(), 2);
}

#[tokio::test]
async fn list_associated_finds_same_workspace() {
    let (repo, _db) = setup().await;

    let mut c1 = make_conversation("ws1");
    c1.extra = r#"{"workspace":"/shared"}"#.to_string();
    c1.id = repo.create(&c1).await.unwrap();

    let mut c2 = make_conversation("ws2");
    c2.extra = r#"{"workspace":"/shared"}"#.to_string();
    c2.id = repo.create(&c2).await.unwrap();

    let mut c3 = make_conversation("ws3");
    c3.extra = r#"{"workspace":"/different"}"#.to_string();
    repo.create(&c3).await.unwrap();

    let assoc = repo.list_associated(USER_ID, c1.id).await.unwrap();
    assert_eq!(assoc.len(), 1);
    assert_eq!(assoc[0].id, c2.id);
}

#[tokio::test]
async fn list_associated_returns_empty_when_no_workspace() {
    let (repo, _db) = setup().await;

    let mut c = make_conversation("no-ws");
    c.extra = r#"{"setting":"value"}"#.to_string();
    c.id = repo.create(&c).await.unwrap();

    let assoc = repo.list_associated(USER_ID, c.id).await.unwrap();
    assert!(assoc.is_empty());
}

// ── Message operations ──────────────────────────────────────────────

#[tokio::test]
async fn message_pagination_and_ordering() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("msgs");
    conv.id = repo.create(&conv).await.unwrap();

    for i in 0..10 {
        let mut msg = make_message(conv.id, &format!("item {i}"));
        msg.created_at = (i + 1) as i64 * 1000;
        repo.insert_message(&msg).await.unwrap();
    }

    // DESC page 1
    let p1 = repo.get_messages(conv.id, 1, 3, SortOrder::Desc).await.unwrap();
    assert_eq!(p1.items.len(), 3);
    assert_eq!(p1.total, 10);
    assert!(p1.has_more);
    assert!(p1.items[0].created_at > p1.items[1].created_at);

    // ASC page 1
    let asc = repo.get_messages(conv.id, 1, 3, SortOrder::Asc).await.unwrap();
    assert!(asc.items[0].created_at < asc.items[1].created_at);
}

#[tokio::test]
async fn update_message_fields() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("msg-update");
    conv.id = repo.create(&conv).await.unwrap();

    let msg = make_message(conv.id, "original");
    repo.insert_message(&msg).await.unwrap();

    repo.update_message(
        &msg.id,
        &MessageRowUpdate {
            content: Some(r#"{"content":"modified"}"#.to_string()),
            hidden: Some(true),
            status: Some(Some("error".to_string())),
        },
    )
    .await
    .unwrap();

    let msgs = repo.get_messages(conv.id, 1, 50, SortOrder::Desc).await.unwrap();
    let updated = &msgs.items[0];
    assert_eq!(updated.content, r#"{"content":"modified"}"#);
    assert!(updated.hidden);
    assert_eq!(updated.status.as_deref(), Some("error"));
}

#[tokio::test]
async fn delete_messages_by_conversation_clears_all() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("msg-delete");
    conv.id = repo.create(&conv).await.unwrap();

    for i in 0..5 {
        let msg = make_message(conv.id, &format!("msg {i}"));
        repo.insert_message(&msg).await.unwrap();
    }

    repo.delete_messages_by_conversation(conv.id).await.unwrap();

    let result = repo.get_messages(conv.id, 1, 50, SortOrder::Desc).await.unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.total, 0);
}

#[tokio::test]
async fn get_message_by_msg_id_triple() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("msg-find");
    conv.id = repo.create(&conv).await.unwrap();

    let mut msg = make_message(conv.id, "findable");
    msg.msg_id = Some("unique_msg_123".to_string());
    msg.r#type = "tool_call".to_string();
    repo.insert_message(&msg).await.unwrap();

    // Match
    let found = repo
        .get_message_by_msg_id(conv.id, "unique_msg_123", "tool_call")
        .await
        .unwrap();
    assert!(found.is_some());

    // Wrong type → None
    let not_found = repo
        .get_message_by_msg_id(conv.id, "unique_msg_123", "text")
        .await
        .unwrap();
    assert!(not_found.is_none());

    // Wrong conv → None
    let not_found = repo
        .get_message_by_msg_id(999_999, "unique_msg_123", "tool_call")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

// ── Message search ──────────────────────────────────────────────────

#[tokio::test]
async fn search_messages_across_conversations() {
    let (repo, _db) = setup().await;

    let mut c1 = make_conversation("search1");
    c1.id = repo.create(&c1).await.unwrap();
    let mut c2 = make_conversation("search2");
    c2.id = repo.create(&c2).await.unwrap();

    let msg1 = make_message(c1.id, "Rust 代码审查报告");
    repo.insert_message(&msg1).await.unwrap();

    let msg2 = make_message(c2.id, "Python 代码审查总结");
    repo.insert_message(&msg2).await.unwrap();

    let msg3 = make_message(c1.id, "unrelated content");
    repo.insert_message(&msg3).await.unwrap();

    let result = repo.search_messages(USER_ID, "审查", 1, 20).await.unwrap();
    assert_eq!(result.total, 2);
    assert_eq!(result.items.len(), 2);

    // Verify conversation names are included
    let names: Vec<_> = result.items.iter().map(|r| &r.conversation_name).collect();
    assert!(names.contains(&&"Conversation search1".to_string()));
    assert!(names.contains(&&"Conversation search2".to_string()));
}

#[tokio::test]
async fn search_messages_empty_result() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("empty-search");
    conv.id = repo.create(&conv).await.unwrap();

    let msg = make_message(conv.id, "hello world");
    repo.insert_message(&msg).await.unwrap();

    let result = repo
        .search_messages(USER_ID, "nonexistent_keyword", 1, 20)
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.total, 0);
    assert!(!result.has_more);
}

#[tokio::test]
async fn search_messages_pagination() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("search-page");
    conv.id = repo.create(&conv).await.unwrap();

    for i in 0..5 {
        let mut msg = make_message(conv.id, &format!("searchable item {i}"));
        msg.created_at = (i + 1) as i64 * 1000;
        repo.insert_message(&msg).await.unwrap();
    }

    let p1 = repo.search_messages(USER_ID, "searchable", 1, 2).await.unwrap();
    assert_eq!(p1.items.len(), 2);
    assert_eq!(p1.total, 5);
    assert!(p1.has_more);

    let p2 = repo.search_messages(USER_ID, "searchable", 2, 2).await.unwrap();
    assert_eq!(p2.items.len(), 2);
    assert!(p2.has_more);

    let p3 = repo.search_messages(USER_ID, "searchable", 3, 2).await.unwrap();
    assert_eq!(p3.items.len(), 1);
    assert!(!p3.has_more);
}

// ── Pinned update flow ──────────────────────────────────────────────

#[tokio::test]
async fn pin_and_unpin_conversation() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("pin-test");
    conv.id = repo.create(&conv).await.unwrap();

    // Pin
    let pin_time = nomifun_common::now_ms();
    repo.update(
        conv.id,
        &ConversationRowUpdate {
            pinned: Some(true),
            pinned_at: Some(Some(pin_time)),
            updated_at: Some(pin_time),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let pinned = repo.get(conv.id).await.unwrap().unwrap();
    assert!(pinned.pinned);
    assert_eq!(pinned.pinned_at, Some(pin_time));

    // Unpin
    let now = nomifun_common::now_ms();
    repo.update(
        conv.id,
        &ConversationRowUpdate {
            pinned: Some(false),
            pinned_at: Some(None),
            updated_at: Some(now),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let unpinned = repo.get(conv.id).await.unwrap().unwrap();
    assert!(!unpinned.pinned);
    assert!(unpinned.pinned_at.is_none());
}

// ── Error cases ─────────────────────────────────────────────────────

#[tokio::test]
async fn update_nonexistent_conversation_returns_not_found() {
    let (repo, _db) = setup().await;
    let err = repo
        .update(
            999_999,
            &ConversationRowUpdate {
                name: Some("x".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
}

#[tokio::test]
async fn delete_nonexistent_conversation_returns_not_found() {
    let (repo, _db) = setup().await;
    let err = repo.delete(999_999).await.unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
}

#[tokio::test]
async fn list_associated_nonexistent_returns_not_found() {
    let (repo, _db) = setup().await;
    let err = repo.list_associated(USER_ID, 999_999).await.unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
}

#[tokio::test]
async fn update_message_nonexistent_returns_not_found() {
    let (repo, _db) = setup().await;
    let err = repo
        .update_message(
            "nonexistent_id",
            &MessageRowUpdate {
                hidden: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
}

// ── Extra field update ──────────────────────────────────────────────

#[tokio::test]
async fn update_extra_replaces_json() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("extra-update");
    conv.id = repo.create(&conv).await.unwrap();

    let now = nomifun_common::now_ms();
    repo.update(
        conv.id,
        &ConversationRowUpdate {
            extra: Some(r#"{"workspace":"/new","flag":true}"#.to_string()),
            updated_at: Some(now),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let found = repo.get(conv.id).await.unwrap().unwrap();
    assert_eq!(found.extra, r#"{"workspace":"/new","flag":true}"#);
}

#[tokio::test]
async fn get_messages_excludes_legacy_cron_and_skill_suggest_rows() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("message-filter");
    conv.id = repo.create(&conv).await.unwrap();

    repo.insert_message(&make_message(conv.id, "visible")).await.unwrap();

    for (id, ty) in [("legacy-cron", "cron_trigger"), ("legacy-skill", "skill_suggest")] {
        repo.insert_message(&MessageRow {
            id: id.into(),
            conversation_id: conv.id.clone(),
            msg_id: None,
            r#type: ty.into(),
            content: "{}".into(),
            position: Some("center".into()),
            status: Some("finish".into()),
            hidden: false,
            created_at: 2000,
        })
        .await
        .unwrap();
    }

    let rows = repo.get_messages(conv.id, 1, 50, SortOrder::Asc).await.unwrap();
    assert_eq!(rows.total, 1);
    assert_eq!(rows.items.len(), 1);
    assert_eq!(rows.items[0].r#type, "text");
}

#[tokio::test]
async fn list_legacy_cron_trigger_messages_returns_only_trigger_rows() {
    let (repo, _db) = setup().await;
    let mut conv = make_conversation("legacy-cron-trigger");
    conv.id = repo.create(&conv).await.unwrap();

    repo.insert_message(&MessageRow {
        id: nomifun_common::generate_prefixed_id("msg"),
        conversation_id: conv.id.clone(),
        msg_id: Some("legacy-trigger".into()),
        r#type: "cron_trigger".into(),
        content: r#"{"cron_job_id":"cron_1","cron_job_name":"Daily Report"}"#.into(),
        position: Some("center".into()),
        status: Some("finish".into()),
        hidden: false,
        created_at: 1000,
    })
    .await
    .unwrap();
    repo.insert_message(&make_message(conv.id, "plain text"))
        .await
        .unwrap();

    let rows = repo.list_legacy_cron_trigger_messages(conv.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].r#type, "cron_trigger");
}

#[tokio::test]
async fn artifact_upsert_list_and_mark_saved() {
    let (repo, db) = setup().await;
    let mut conv = make_conversation("artifact-row");
    conv.id = repo.create(&conv).await.unwrap();
    seed_cron_job(db.pool(), "cron_1").await;

    let inserted = repo.upsert_artifact(&make_artifact(conv.id)).await.unwrap();
    assert_eq!(inserted.status, "pending");
    let artifact_id = inserted.id;

    let listed = repo.list_artifacts(conv.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, artifact_id);

    let dismissed = repo
        .update_artifact_status(conv.id, artifact_id, "dismissed", 2000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dismissed.status, "dismissed");
    assert_eq!(dismissed.updated_at, 2000);

    let saved = repo.mark_skill_suggest_artifacts_saved("cron_1", 3000).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].status, "saved");
    assert_eq!(saved[0].updated_at, 3000);
}

#[tokio::test]
async fn delete_artifacts_by_conversation_removes_rows() {
    let (repo, db) = setup().await;
    let mut conv = make_conversation("artifact-delete");
    conv.id = repo.create(&conv).await.unwrap();
    seed_cron_job(db.pool(), "cron_1").await;

    repo.upsert_artifact(&make_artifact(conv.id)).await.unwrap();

    repo.delete_artifacts_by_conversation(conv.id).await.unwrap();

    let listed = repo.list_artifacts(conv.id).await.unwrap();
    assert!(listed.is_empty());
}

// ── User isolation ──────────────────────────────────────────────────

#[tokio::test]
async fn list_paginated_scoped_to_user() {
    let (repo, db) = setup().await;

    // Create a second user
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_2', 'other', 'hash', 1000, 1000)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let c1 = make_conversation("user1-conv");
    repo.create(&c1).await.unwrap();

    let mut c2 = make_conversation("user2-conv");
    c2.user_id = "user_2".to_string();
    repo.create(&c2).await.unwrap();

    // User 1 only sees their own
    let result = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                limit: 20,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].user_id, USER_ID);
}
