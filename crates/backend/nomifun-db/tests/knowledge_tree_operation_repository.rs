use nomifun_common::{KnowledgeBaseId, validate_uuidv7};
use nomifun_db::{
    CommitKnowledgeTreeOperationParams, DbError, IKnowledgeRepository,
    IKnowledgeTreeOperationRepository, KnowledgeBaseRow, KnowledgeTreeEventStatus,
    KnowledgeTreeOperationPageCursor, KnowledgeTreeOperationState,
    PrepareKnowledgeTreeOperationParams,
    SqliteKnowledgeRepository, SqliteKnowledgeTreeOperationRepository, init_database_memory,
};
use serde_json::json;
use sqlx::migrate::{Migrate, Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn migrate_through(pool: &sqlx::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    for migration in MIGRATOR.iter() {
        if migration.version <= maximum_version && !applied.contains(&migration.version) {
            connection.apply(migration).await.unwrap();
        }
    }
}

fn base(knowledge_base_id: &KnowledgeBaseId) -> KnowledgeBaseRow {
    KnowledgeBaseRow {
        id: 0,
        knowledge_base_id: knowledge_base_id.to_string(),
        name: "journal fixture".into(),
        description: String::new(),
        root_path: format!("/tmp/{knowledge_base_id}"),
        managed: true,
        tree_access: "editable".into(),
        extra: "{}".into(),
        created_at: 1,
        updated_at: 1,
        tags: None,
    }
}

fn prepare(
    knowledge_base_id: &KnowledgeBaseId,
    request_id: &str,
) -> PrepareKnowledgeTreeOperationParams {
    PrepareKnowledgeTreeOperationParams {
        knowledge_base_id: knowledge_base_id.clone(),
        request_id: request_id.into(),
        fingerprint: "a".repeat(64),
        source_rel_path: "Docs/A.md".into(),
        destination_rel_path: "Archive/A.md".into(),
        source_fs_identity: Some("test-fs:1".into()),
        created_at: 10,
    }
}

#[tokio::test]
async fn migration_adds_the_journal_without_rewriting_existing_bases() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&pool, 53).await;
    let knowledge_base_id = KnowledgeBaseId::new();
    sqlx::query(
        "INSERT INTO knowledge_bases \
            (knowledge_base_id, name, description, root_path, managed, extra, created_at, updated_at) \
         VALUES (?, 'existing', '', '/tmp/existing', 1, '{}', 1, 1)",
    )
    .bind(knowledge_base_id.as_str())
    .execute(&pool)
    .await
    .unwrap();
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'table' AND name = 'knowledge_tree_operations'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, 0);

    migrate_through(&pool, 54).await;

    let preserved: String = sqlx::query_scalar(
        "SELECT name FROM knowledge_bases WHERE knowledge_base_id = ?",
    )
    .bind(knowledge_base_id.as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(preserved, "existing");
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'table' AND name = 'knowledge_tree_operations'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after, 1);
    let identity_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_tree_operations') \
         WHERE name = 'source_fs_identity'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(identity_column, 1);
}

#[tokio::test]
async fn prepare_is_durable_idempotent_and_rejects_request_reuse() {
    let database = init_database_memory().await.unwrap();
    let base_repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let repository =
        SqliteKnowledgeTreeOperationRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    base_repository
        .insert_base(&base(&knowledge_base_id))
        .await
        .unwrap();

    let params = prepare(&knowledge_base_id, "drag-once");
    let first = repository.prepare_operation(&params).await.unwrap();
    assert!(first.created);
    assert_eq!(first.operation.state, KnowledgeTreeOperationState::Prepared);
    assert_eq!(first.operation.event_status, KnowledgeTreeEventStatus::None);
    assert_eq!(
        first.operation.source_fs_identity.as_deref(),
        Some("test-fs:1")
    );
    validate_uuidv7(first.operation.operation_id.as_str()).unwrap();

    let replay = repository.prepare_operation(&params).await.unwrap();
    assert!(!replay.created);
    assert_eq!(replay.operation.operation_id, first.operation.operation_id);

    let mut reused = params.clone();
    reused.fingerprint = "b".repeat(64);
    let error = repository.prepare_operation(&reused).await.unwrap_err();
    assert!(matches!(error, DbError::Conflict(message) if message.contains("different operation")));

    let by_request = repository
        .load_by_request(&knowledge_base_id, "drag-once")
        .await
        .unwrap()
        .unwrap();
    let by_operation = repository
        .load_by_operation(&first.operation.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_request, by_operation);
}

#[tokio::test]
async fn receipt_and_pending_event_commit_atomically_and_publish_idempotently() {
    let database = init_database_memory().await.unwrap();
    let base_repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let repository =
        SqliteKnowledgeTreeOperationRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    base_repository
        .insert_base(&base(&knowledge_base_id))
        .await
        .unwrap();
    let prepared = repository
        .prepare_operation(&prepare(&knowledge_base_id, "commit-once"))
        .await
        .unwrap()
        .operation;

    let filesystem_committed = repository
        .mark_filesystem_committed(&prepared.operation_id, 11)
        .await
        .unwrap();
    assert_eq!(
        filesystem_committed.state,
        KnowledgeTreeOperationState::FilesystemCommitted
    );

    let invalid_commit = CommitKnowledgeTreeOperationParams {
        operation_id: prepared.operation_id.clone(),
        receipt: json!({"oldPath": "Docs/A.md", "newPath": "Archive/A.md"}),
        event_payload: json!(["not", "an", "event", "object"]),
        committed_at: 12,
    };
    assert!(matches!(
        repository.commit_operation(&invalid_commit).await,
        Err(DbError::Conflict(_))
    ));
    let unchanged = repository
        .load_by_operation(&prepared.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.state,
        KnowledgeTreeOperationState::FilesystemCommitted
    );
    assert!(unchanged.receipt_json.is_none());
    assert_eq!(unchanged.event_status, KnowledgeTreeEventStatus::None);

    let commit = CommitKnowledgeTreeOperationParams {
        operation_id: prepared.operation_id.clone(),
        receipt: json!({"oldPath": "Docs/A.md", "newPath": "Archive/A.md"}),
        event_payload: json!({"kind": "knowledge.tree-changed", "treeRevision": 2}),
        committed_at: 12,
    };
    let committed = repository.commit_operation(&commit).await.unwrap();
    assert_eq!(committed.state, KnowledgeTreeOperationState::Committed);
    assert!(committed.receipt_json.is_some());
    assert_eq!(committed.event_status, KnowledgeTreeEventStatus::Pending);
    assert!(committed.event_payload_json.is_some());

    let pending = repository.list_pending_events(10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].operation_id, prepared.operation_id);
    assert!(repository.list_pending_recovery(10).await.unwrap().is_empty());

    let published = repository
        .mark_event_published(&prepared.operation_id, 13)
        .await
        .unwrap();
    assert_eq!(published.event_status, KnowledgeTreeEventStatus::Published);
    assert_eq!(published.event_published_at, Some(13));
    assert!(repository.list_pending_events(10).await.unwrap().is_empty());

    let publish_replay = repository
        .mark_event_published(&prepared.operation_id, 99)
        .await
        .unwrap();
    assert_eq!(publish_replay.event_published_at, Some(13));
    let commit_replay = repository.commit_operation(&commit).await.unwrap();
    assert_eq!(commit_replay.event_status, KnowledgeTreeEventStatus::Published);

    let conflicting_commit = CommitKnowledgeTreeOperationParams {
        receipt: json!({"oldPath": "another.md"}),
        ..commit
    };
    assert!(matches!(
        repository.commit_operation(&conflicting_commit).await,
        Err(DbError::Conflict(_))
    ));
}

#[tokio::test]
async fn recovery_scan_includes_every_nonterminal_crash_window_and_can_resume() {
    let database = init_database_memory().await.unwrap();
    let base_repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let repository =
        SqliteKnowledgeTreeOperationRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    base_repository
        .insert_base(&base(&knowledge_base_id))
        .await
        .unwrap();

    let prepared = repository
        .prepare_operation(&prepare(&knowledge_base_id, "prepared-window"))
        .await
        .unwrap()
        .operation;
    let mut filesystem_params = prepare(&knowledge_base_id, "filesystem-window");
    filesystem_params.fingerprint = "b".repeat(64);
    filesystem_params.source_rel_path = "Docs/B.md".into();
    filesystem_params.destination_rel_path = "Archive/B.md".into();
    let filesystem = repository
        .prepare_operation(&filesystem_params)
        .await
        .unwrap()
        .operation;
    repository
        .mark_filesystem_committed(&filesystem.operation_id, 11)
        .await
        .unwrap();
    let mut recovery_params = prepare(&knowledge_base_id, "recovery-window");
    recovery_params.fingerprint = "c".repeat(64);
    recovery_params.source_rel_path = "Docs/C.md".into();
    recovery_params.destination_rel_path = "Archive/C.md".into();
    let recovery = repository
        .prepare_operation(&recovery_params)
        .await
        .unwrap()
        .operation;
    repository
        .mark_needs_recovery(&recovery.operation_id, "destination state is ambiguous", 11)
        .await
        .unwrap();

    let pending = repository.list_pending_recovery(10).await.unwrap();
    assert_eq!(pending.len(), 3);
    assert!(pending.iter().any(|row| row.operation_id == prepared.operation_id));
    assert!(pending.iter().any(|row| {
        row.operation_id == filesystem.operation_id
            && row.state == KnowledgeTreeOperationState::FilesystemCommitted
    }));
    assert!(pending.iter().any(|row| {
        row.operation_id == recovery.operation_id
            && row.state == KnowledgeTreeOperationState::NeedsRecovery
    }));

    let mut paged_ids = Vec::new();
    let mut cursor: Option<KnowledgeTreeOperationPageCursor> = None;
    loop {
        let page = repository
            .list_pending_recovery_after(1, cursor.as_ref())
            .await
            .unwrap();
        let Some(row) = page.into_iter().next() else {
            break;
        };
        cursor = Some(KnowledgeTreeOperationPageCursor {
            timestamp: row.created_at,
            operation_id: row.operation_id.clone(),
        });
        paged_ids.push(row.operation_id);
    }
    assert_eq!(
        paged_ids,
        pending
            .iter()
            .map(|row| row.operation_id.clone())
            .collect::<Vec<_>>()
    );

    let resumed = repository
        .mark_filesystem_committed(&recovery.operation_id, 12)
        .await
        .unwrap();
    assert_eq!(
        resumed.state,
        KnowledgeTreeOperationState::FilesystemCommitted
    );
    assert!(resumed.error_message.is_none());
}

#[tokio::test]
async fn deleting_a_base_cleans_its_journal_and_outbox_rows() {
    let database = init_database_memory().await.unwrap();
    let base_repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let repository =
        SqliteKnowledgeTreeOperationRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    base_repository
        .insert_base(&base(&knowledge_base_id))
        .await
        .unwrap();
    let operation = repository
        .prepare_operation(&prepare(&knowledge_base_id, "delete-base"))
        .await
        .unwrap()
        .operation;

    base_repository
        .delete_base(knowledge_base_id.as_str())
        .await
        .unwrap();

    assert!(
        repository
            .load_by_operation(&operation.operation_id)
            .await
            .unwrap()
            .is_none()
    );
}
