use nomifun_common::{
    KnowledgeBaseId, KnowledgeEntryId, KnowledgeSourceId, KnowledgeSourceItemId,
};
use nomifun_db::{
    BindManagedKnowledgeEntryParams, CreateKnowledgeSourceItemParams, DbError,
    EnsureKnowledgeSourceParams, IKnowledgeEntryRepository, IKnowledgeRepository,
    IKnowledgeSourceRepository, KnowledgeBaseRow, KnowledgeEntryProvenanceRelationship,
    KnowledgeSourceItemSyncStatus, KnowledgeSourceKind, KnowledgeSourceMode,
    KnowledgeSourceState, RecordKnowledgeEntryCopyParams,
    RecordKnowledgeSourceSyncFailureParams, RecordKnowledgeSourceSyncSuccessParams,
    SqliteKnowledgeRepository, StageKnowledgeSourcePublicationParams,
    UpdateKnowledgeSourceItemParams, UpdateKnowledgeSourceParams, UpsertKnowledgeEntryParams,
    init_database_memory, validate_id_data_contract,
};
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

fn base(knowledge_base_id: &KnowledgeBaseId, name: &str) -> KnowledgeBaseRow {
    KnowledgeBaseRow {
        id: 0,
        knowledge_base_id: knowledge_base_id.to_string(),
        name: name.into(),
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

fn entry(
    knowledge_base_id: &KnowledgeBaseId,
    knowledge_entry_id: &KnowledgeEntryId,
    name: &str,
    kind: &str,
) -> UpsertKnowledgeEntryParams {
    UpsertKnowledgeEntryParams {
        knowledge_entry_id: knowledge_entry_id.clone(),
        knowledge_base_id: knowledge_base_id.clone(),
        parent_entry_id: None,
        name: name.into(),
        kind: kind.into(),
        origin: "user".into(),
        rel_path: name.into(),
        portable_rel_path: name.to_ascii_lowercase(),
        fs_identity: Some(format!("identity:{knowledge_entry_id}")),
        content_hash: (kind == "file").then(|| "0".repeat(64)),
        revision: 0,
        deleted_at: None,
        created_at: 2,
        updated_at: 2,
    }
}

fn source_params(
    knowledge_base_id: &KnowledgeBaseId,
    knowledge_source_id: &KnowledgeSourceId,
    default_parent_entry_id: Option<&KnowledgeEntryId>,
) -> EnsureKnowledgeSourceParams {
    EnsureKnowledgeSourceParams {
        knowledge_source_id: knowledge_source_id.clone(),
        knowledge_base_id: knowledge_base_id.clone(),
        kind: KnowledgeSourceKind::Url,
        mode: KnowledgeSourceMode::Snapshot,
        default_parent_entry_id: default_parent_entry_id.cloned(),
        created_at: 10,
    }
}

fn item_params(
    knowledge_source_id: &KnowledgeSourceId,
    knowledge_source_item_id: &KnowledgeSourceItemId,
    url: &str,
    ordinal: i64,
) -> CreateKnowledgeSourceItemParams {
    CreateKnowledgeSourceItemParams {
        knowledge_source_item_id: knowledge_source_item_id.clone(),
        knowledge_source_id: knowledge_source_id.clone(),
        requested_url: url.into(),
        normalized_url: url.into(),
        final_url: None,
        rendered: false,
        title: None,
        ordinal,
        state: KnowledgeSourceState::Active,
        sync_status: KnowledgeSourceItemSyncStatus::Pending,
        etag: None,
        http_last_modified: None,
        last_attempt_at: None,
        last_success_at: None,
        last_error: None,
        last_published_hash: None,
        pending_published_hash: None,
        pending_final_url: None,
        pending_title: None,
        pending_publication_at: None,
        removed_at: None,
        created_at: 10,
    }
}

#[tokio::test]
async fn migration_adds_normalized_source_tables_without_rewriting_legacy_extra() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&pool, 54).await;
    let knowledge_base_id = KnowledgeBaseId::new();
    let legacy_extra = serde_json::json!({
        "tree_access": "editable",
        "source": {
            "kind": "url",
            "mode": "snapshot",
            "entries": [{"url": "https://example.test/docs"}]
        }
    })
    .to_string();
    sqlx::query(
        "INSERT INTO knowledge_bases (\
            knowledge_base_id, name, description, root_path, managed, extra, created_at, updated_at\
         ) VALUES (?, 'legacy', '', '/tmp/legacy', 1, ?, 1, 1)",
    )
    .bind(knowledge_base_id.as_str())
    .bind(&legacy_extra)
    .execute(&pool)
    .await
    .unwrap();

    migrate_through(&pool, 55).await;

    for table in [
        "knowledge_sources",
        "knowledge_source_items",
        "knowledge_entry_provenance",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "missing {table}");
    }
    let preserved: String =
        sqlx::query_scalar("SELECT extra FROM knowledge_bases WHERE knowledge_base_id = ?")
            .bind(knowledge_base_id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(preserved, legacy_extra);
    let foreign_keys: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('knowledge_sources')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(foreign_keys, 0);
    let pending_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_source_items') \
         WHERE name LIKE 'pending_%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending_before, 0, "v55 is the immutable base source schema");

    migrate_through(&pool, 56).await;
    let pending_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_source_items') \
         WHERE name IN (\
             'pending_published_hash', 'pending_final_url', \
             'pending_title', 'pending_publication_at'\
         )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending_after, 4);
}

#[tokio::test]
async fn source_ensure_is_idempotent_and_configuration_updates_use_revision_cas() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&knowledge_base_id, "source"))
        .await
        .unwrap();
    let folder_id = KnowledgeEntryId::new();
    repository
        .upsert_entry(&entry(&knowledge_base_id, &folder_id, "Web", "directory"))
        .await
        .unwrap();

    let source_id = KnowledgeSourceId::new();
    let created = repository
        .ensure_source(&source_params(
            &knowledge_base_id,
            &source_id,
            Some(&folder_id),
        ))
        .await
        .unwrap();
    assert!(created.created);
    assert_eq!(created.source.revision, 0);
    assert_eq!(created.source.default_parent_entry_id, Some(folder_id.clone()));

    let replay_id = KnowledgeSourceId::new();
    let replay = repository
        .ensure_source(&EnsureKnowledgeSourceParams {
            knowledge_source_id: replay_id,
            knowledge_base_id: knowledge_base_id.clone(),
            kind: KnowledgeSourceKind::Url,
            mode: KnowledgeSourceMode::Live,
            default_parent_entry_id: None,
            created_at: 11,
        })
        .await
        .unwrap();
    assert!(!replay.created);
    assert_eq!(replay.source.knowledge_source_id, source_id);
    assert_eq!(replay.source.mode, KnowledgeSourceMode::Snapshot);

    let updated = repository
        .update_source(&UpdateKnowledgeSourceParams {
            knowledge_source_id: source_id.clone(),
            expected_revision: 0,
            mode: KnowledgeSourceMode::Live,
            state: KnowledgeSourceState::Paused,
            default_parent_entry_id: Some(folder_id.clone()),
            removed_at: None,
            updated_at: 12,
        })
        .await
        .unwrap();
    assert_eq!(updated.revision, 1);
    assert_eq!(updated.state, KnowledgeSourceState::Paused);
    let stale = repository
        .update_source(&UpdateKnowledgeSourceParams {
            knowledge_source_id: source_id.clone(),
            expected_revision: 0,
            mode: KnowledgeSourceMode::Snapshot,
            state: KnowledgeSourceState::Active,
            default_parent_entry_id: None,
            removed_at: None,
            updated_at: 13,
        })
        .await
        .unwrap_err();
    assert!(matches!(stale, DbError::Conflict(_)), "{stale:?}");
}

#[tokio::test]
async fn projection_tombstones_clear_source_default_parent_in_the_same_transaction() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&knowledge_base_id, "placement"))
        .await
        .unwrap();
    let first_folder_id = KnowledgeEntryId::new();
    let second_folder_id = KnowledgeEntryId::new();
    repository
        .upsert_entry(&entry(
            &knowledge_base_id,
            &first_folder_id,
            "First",
            "directory",
        ))
        .await
        .unwrap();
    repository
        .upsert_entry(&entry(
            &knowledge_base_id,
            &second_folder_id,
            "Second",
            "directory",
        ))
        .await
        .unwrap();
    let source_id = KnowledgeSourceId::new();
    repository
        .ensure_source(&source_params(
            &knowledge_base_id,
            &source_id,
            Some(&first_folder_id),
        ))
        .await
        .unwrap();

    repository
        .replace_projection(
            &knowledge_base_id,
            Some(2),
            &[entry(
                &knowledge_base_id,
                &second_folder_id,
                "Second",
                "directory",
            )],
        )
        .await
        .unwrap();
    let source = repository.get_source(&source_id).await.unwrap().unwrap();
    assert_eq!(source.default_parent_entry_id, None);
    assert_eq!(source.revision, 1);

    let source = repository
        .update_source(&UpdateKnowledgeSourceParams {
            knowledge_source_id: source_id.clone(),
            expected_revision: source.revision,
            mode: KnowledgeSourceMode::Snapshot,
            state: KnowledgeSourceState::Active,
            default_parent_entry_id: Some(second_folder_id.clone()),
            removed_at: None,
            updated_at: 20,
        })
        .await
        .unwrap();
    assert_eq!(source.revision, 2);
    repository
        .soft_delete_entry_subtree(&knowledge_base_id, &second_folder_id, 0, 21)
        .await
        .unwrap();
    let source = repository.get_source(&source_id).await.unwrap().unwrap();
    assert_eq!(source.default_parent_entry_id, None);
    assert_eq!(source.revision, 3);
}

#[tokio::test]
async fn source_items_enforce_live_url_and_ordinal_uniqueness_and_record_sync_history() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&knowledge_base_id, "items"))
        .await
        .unwrap();
    let source_id = KnowledgeSourceId::new();
    repository
        .ensure_source(&source_params(&knowledge_base_id, &source_id, None))
        .await
        .unwrap();

    let item_id = KnowledgeSourceItemId::new();
    let item = repository
        .create_source_item(&item_params(
            &source_id,
            &item_id,
            "https://example.test/docs",
            0,
        ))
        .await
        .unwrap();
    assert_eq!(item.revision, 0);
    assert_eq!(
        repository
            .get_live_source_item_by_url(&source_id, "https://example.test/docs")
            .await
            .unwrap()
            .unwrap()
            .knowledge_source_item_id,
        item_id
    );
    assert_eq!(
        repository
            .list_source_items(&source_id, false)
            .await
            .unwrap()
            .len(),
        1
    );
    let premature_source_remove = repository
        .update_source(&UpdateKnowledgeSourceParams {
            knowledge_source_id: source_id.clone(),
            expected_revision: 0,
            mode: KnowledgeSourceMode::Snapshot,
            state: KnowledgeSourceState::Removed,
            default_parent_entry_id: None,
            removed_at: Some(11),
            updated_at: 11,
        })
        .await
        .unwrap_err();
    assert!(matches!(premature_source_remove, DbError::Conflict(_)));

    let duplicate_url = repository
        .create_source_item(&item_params(
            &source_id,
            &KnowledgeSourceItemId::new(),
            "https://example.test/docs",
            1,
        ))
        .await
        .unwrap_err();
    assert!(matches!(duplicate_url, DbError::Conflict(_)));
    let duplicate_ordinal = repository
        .create_source_item(&item_params(
            &source_id,
            &KnowledgeSourceItemId::new(),
            "https://example.test/other",
            0,
        ))
        .await
        .unwrap_err();
    assert!(matches!(duplicate_ordinal, DbError::Conflict(_)));

    let syncing = repository
        .record_sync_attempt(&item_id, 0, 20)
        .await
        .unwrap();
    assert_eq!(syncing.revision, 1);
    assert_eq!(syncing.sync_status, KnowledgeSourceItemSyncStatus::Syncing);
    let synced = repository
        .record_sync_success(&RecordKnowledgeSourceSyncSuccessParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: 1,
            final_url: Some("https://example.test/final".into()),
            title: Some("Docs".into()),
            etag: Some("\"v1\"".into()),
            http_last_modified: Some("Wed, 01 Jan 2025 00:00:00 GMT".into()),
            last_published_hash: "a".repeat(64),
            succeeded_at: 21,
        })
        .await
        .unwrap();
    assert_eq!(synced.revision, 2);
    assert_eq!(synced.last_success_at, Some(21));
    assert_eq!(synced.last_published_hash.as_deref(), Some("a".repeat(64).as_str()));

    let stale = repository.record_sync_attempt(&item_id, 1, 22).await.unwrap_err();
    assert!(matches!(stale, DbError::Conflict(_)));
    let syncing = repository.record_sync_attempt(&item_id, 2, 22).await.unwrap();
    let failed = repository
        .record_sync_failure(&RecordKnowledgeSourceSyncFailureParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: syncing.revision,
            status: KnowledgeSourceItemSyncStatus::Failed,
            error: "upstream timeout".into(),
            failed_at: 23,
        })
        .await
        .unwrap();
    assert_eq!(failed.sync_status, KnowledgeSourceItemSyncStatus::Failed);
    assert_eq!(failed.last_success_at, Some(21));
    assert_eq!(failed.last_error.as_deref(), Some("upstream timeout"));

    let syncing_again = repository
        .record_sync_attempt(&item_id, failed.revision, 24)
        .await
        .unwrap();
    let conflicted = repository
        .record_sync_failure(&RecordKnowledgeSourceSyncFailureParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: syncing_again.revision,
            status: KnowledgeSourceItemSyncStatus::Conflicted,
            error: "local content changed".into(),
            failed_at: 25,
        })
        .await
        .unwrap();
    assert_eq!(
        conflicted.sync_status,
        KnowledgeSourceItemSyncStatus::Conflicted
    );

    let edited = repository
        .update_source_item(&UpdateKnowledgeSourceItemParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: conflicted.revision,
            requested_url: "https://example.test/docs-v2".into(),
            normalized_url: "https://example.test/docs-v2".into(),
            final_url: conflicted.final_url.clone(),
            rendered: true,
            title: Some("Docs v2".into()),
            ordinal: 2,
            state: KnowledgeSourceState::Active,
            sync_status: conflicted.sync_status,
            etag: conflicted.etag.clone(),
            http_last_modified: conflicted.http_last_modified.clone(),
            last_attempt_at: conflicted.last_attempt_at,
            last_success_at: conflicted.last_success_at,
            last_error: conflicted.last_error.clone(),
            last_published_hash: conflicted.last_published_hash.clone(),
            pending_published_hash: conflicted.pending_published_hash.clone(),
            pending_final_url: conflicted.pending_final_url.clone(),
            pending_title: conflicted.pending_title.clone(),
            pending_publication_at: conflicted.pending_publication_at,
            removed_at: None,
            updated_at: 26,
        })
        .await
        .unwrap();
    assert_eq!(edited.revision, conflicted.revision + 1);
    assert!(edited.rendered);
    assert!(repository
        .get_live_source_item_by_url(&source_id, "https://example.test/docs")
        .await
        .unwrap()
        .is_none());

    let removed = repository
        .remove_source_item(&item_id, edited.revision, 27)
        .await
        .unwrap();
    assert_eq!(removed.state, KnowledgeSourceState::Removed);
    let replacement = repository
        .create_source_item(&item_params(
            &source_id,
            &KnowledgeSourceItemId::new(),
            "https://example.test/docs",
            0,
        ))
        .await
        .unwrap();
    assert_eq!(replacement.state, KnowledgeSourceState::Active);
}

#[tokio::test]
async fn staged_publication_survives_reload_and_settles_with_hash_cas() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&knowledge_base_id, "staged publication"))
        .await
        .unwrap();
    let source_id = KnowledgeSourceId::new();
    repository
        .ensure_source(&source_params(&knowledge_base_id, &source_id, None))
        .await
        .unwrap();
    let item_id = KnowledgeSourceItemId::new();
    repository
        .create_source_item(&item_params(
            &source_id,
            &item_id,
            "https://example.test/staged",
            0,
        ))
        .await
        .unwrap();

    let attempted = repository
        .record_sync_attempt(&item_id, 0, 20)
        .await
        .unwrap();
    let staged_hash = "b".repeat(64);
    let staged = repository
        .stage_sync_publication(&StageKnowledgeSourcePublicationParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: attempted.revision,
            pending_published_hash: staged_hash.clone(),
            pending_final_url: Some("https://example.test/final".into()),
            pending_title: Some("Staged title".into()),
            staged_at: 21,
        })
        .await
        .unwrap();
    assert_eq!(staged.revision, attempted.revision + 1);

    // A fresh repository handle models process restart: pending intent is
    // durable row state, not process-local coordination.
    let reloaded_repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let reloaded = reloaded_repository
        .get_source_item(&item_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.pending_published_hash.as_deref(), Some(staged_hash.as_str()));
    assert_eq!(reloaded.pending_publication_at, Some(21));
    assert_eq!(reloaded.pending_title.as_deref(), Some("Staged title"));

    let stale_stage = reloaded_repository
        .stage_sync_publication(&StageKnowledgeSourcePublicationParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: attempted.revision,
            pending_published_hash: "c".repeat(64),
            pending_final_url: None,
            pending_title: None,
            staged_at: 22,
        })
        .await
        .unwrap_err();
    assert!(matches!(stale_stage, DbError::Conflict(_)));
    let mismatched_success = reloaded_repository
        .record_sync_success(&RecordKnowledgeSourceSyncSuccessParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: staged.revision,
            final_url: Some("https://example.test/final".into()),
            title: Some("Staged title".into()),
            etag: None,
            http_last_modified: None,
            last_published_hash: "f".repeat(64),
            succeeded_at: 22,
        })
        .await
        .unwrap_err();
    assert!(matches!(mismatched_success, DbError::Conflict(_)));
    assert_eq!(
        reloaded_repository
            .get_source_item(&item_id)
            .await
            .unwrap()
            .unwrap()
            .pending_published_hash
            .as_deref(),
        Some(staged_hash.as_str())
    );

    let succeeded = reloaded_repository
        .record_sync_success(&RecordKnowledgeSourceSyncSuccessParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: staged.revision,
            final_url: Some("https://example.test/final".into()),
            title: Some("Staged title".into()),
            etag: Some("\"v2\"".into()),
            http_last_modified: None,
            last_published_hash: staged_hash.clone(),
            succeeded_at: 22,
        })
        .await
        .unwrap();
    assert_eq!(succeeded.last_published_hash.as_deref(), Some(staged_hash.as_str()));
    assert!(succeeded.pending_published_hash.is_none());
    assert!(succeeded.pending_publication_at.is_none());

    let attempted = reloaded_repository
        .record_sync_attempt(&item_id, succeeded.revision, 23)
        .await
        .unwrap();
    let staged = reloaded_repository
        .stage_sync_publication(&StageKnowledgeSourcePublicationParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: attempted.revision,
            pending_published_hash: "c".repeat(64),
            pending_final_url: None,
            pending_title: None,
            staged_at: 24,
        })
        .await
        .unwrap();
    let failed = reloaded_repository
        .record_sync_failure(&RecordKnowledgeSourceSyncFailureParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: staged.revision,
            status: KnowledgeSourceItemSyncStatus::Failed,
            error: "filesystem publication failed".into(),
            failed_at: 25,
        })
        .await
        .unwrap();
    assert!(failed.pending_published_hash.is_none());
    assert!(failed.pending_final_url.is_none());

    let attempted = reloaded_repository
        .record_sync_attempt(&item_id, failed.revision, 26)
        .await
        .unwrap();
    let staged = reloaded_repository
        .stage_sync_publication(&StageKnowledgeSourcePublicationParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: attempted.revision,
            pending_published_hash: "d".repeat(64),
            pending_final_url: None,
            pending_title: None,
            staged_at: 27,
        })
        .await
        .unwrap();
    let restarted_attempt = reloaded_repository
        .record_sync_attempt(&item_id, staged.revision, 28)
        .await
        .unwrap();
    assert!(restarted_attempt.pending_published_hash.is_none());
    let staged = reloaded_repository
        .stage_sync_publication(&StageKnowledgeSourcePublicationParams {
            knowledge_source_item_id: item_id.clone(),
            expected_revision: restarted_attempt.revision,
            pending_published_hash: "e".repeat(64),
            pending_final_url: None,
            pending_title: None,
            staged_at: 29,
        })
        .await
        .unwrap();
    let removed = reloaded_repository
        .remove_source_item(&item_id, staged.revision, 30)
        .await
        .unwrap();
    assert_eq!(removed.state, KnowledgeSourceState::Removed);
    assert_eq!(removed.sync_status, KnowledgeSourceItemSyncStatus::Missing);
    assert!(removed.pending_published_hash.is_none());
    assert!(removed.pending_publication_at.is_none());
}

#[tokio::test]
async fn managed_detached_and_copy_provenance_are_identity_based_and_transactional() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&knowledge_base_id, "provenance"))
        .await
        .unwrap();
    let managed_entry_id = KnowledgeEntryId::new();
    let copy_entry_id = KnowledgeEntryId::new();
    let second_entry_id = KnowledgeEntryId::new();
    for (entry_id, name) in [
        (&managed_entry_id, "managed.md"),
        (&copy_entry_id, "copy.md"),
        (&second_entry_id, "second.md"),
    ] {
        repository
            .upsert_entry(&entry(&knowledge_base_id, entry_id, name, "file"))
            .await
            .unwrap();
    }
    let source_id = KnowledgeSourceId::new();
    repository
        .ensure_source(&source_params(&knowledge_base_id, &source_id, None))
        .await
        .unwrap();
    let item_id = KnowledgeSourceItemId::new();
    repository
        .create_source_item(&item_params(
            &source_id,
            &item_id,
            "https://example.test/page",
            0,
        ))
        .await
        .unwrap();

    let managed = repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: managed_entry_id.clone(),
            knowledge_source_item_id: item_id.clone(),
            created_at: 20,
        })
        .await
        .unwrap();
    assert_eq!(managed.relationship, KnowledgeEntryProvenanceRelationship::Managed);
    let replay = repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: managed_entry_id.clone(),
            knowledge_source_item_id: item_id.clone(),
            created_at: 21,
        })
        .await
        .unwrap();
    assert_eq!(replay.id, managed.id);

    let second_managed = repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: second_entry_id,
            knowledge_source_item_id: item_id.clone(),
            created_at: 21,
        })
        .await
        .unwrap_err();
    assert!(matches!(second_managed, DbError::Conflict(_)));

    let copy = repository
        .record_entry_copy(&RecordKnowledgeEntryCopyParams {
            knowledge_entry_id: copy_entry_id.clone(),
            knowledge_source_item_id: item_id.clone(),
            derived_from_entry_id: managed_entry_id.clone(),
            created_at: 22,
        })
        .await
        .unwrap();
    assert_eq!(copy.relationship, KnowledgeEntryProvenanceRelationship::Copy);
    assert_eq!(copy.derived_from_entry_id, Some(managed_entry_id.clone()));

    let blocked_remove = repository.remove_source_item(&item_id, 0, 23).await.unwrap_err();
    assert!(matches!(blocked_remove, DbError::Conflict(_)));
    let detached = repository
        .detach_managed_entry(&managed_entry_id, managed.revision, 23)
        .await
        .unwrap();
    assert_eq!(detached.relationship, KnowledgeEntryProvenanceRelationship::Detached);
    assert_eq!(detached.revision, 1);
    assert_eq!(detached.detached_at, Some(23));
    let paused_item = repository.get_source_item(&item_id).await.unwrap().unwrap();
    assert_eq!(paused_item.state, KnowledgeSourceState::Paused);
    assert_eq!(paused_item.revision, 1);

    let rows = repository
        .list_entry_provenance_for_source(&source_id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        repository
            .list_entry_provenance_for_item(&item_id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        repository
            .get_entry_provenance(&copy_entry_id)
            .await
            .unwrap()
            .unwrap()
            .relationship,
        KnowledgeEntryProvenanceRelationship::Copy
    );
    assert!(repository
        .get_managed_entry_provenance(&item_id)
        .await
        .unwrap()
        .is_none());
    let removed = repository
        .remove_source_item(&item_id, paused_item.revision, 24)
        .await
        .unwrap();
    assert_eq!(removed.state, KnowledgeSourceState::Removed);
    let removed_source = repository
        .update_source(&UpdateKnowledgeSourceParams {
            knowledge_source_id: source_id.clone(),
            expected_revision: 0,
            mode: KnowledgeSourceMode::Snapshot,
            state: KnowledgeSourceState::Removed,
            default_parent_entry_id: None,
            removed_at: Some(25),
            updated_at: 25,
        })
        .await
        .unwrap();
    assert_eq!(removed_source.state, KnowledgeSourceState::Removed);
    assert_eq!(removed_source.revision, 1);
    let removed_replay = repository
        .ensure_source(&EnsureKnowledgeSourceParams {
            knowledge_source_id: source_id,
            knowledge_base_id: knowledge_base_id.clone(),
            kind: KnowledgeSourceKind::Url,
            mode: KnowledgeSourceMode::Live,
            default_parent_entry_id: None,
            created_at: 26,
        })
        .await
        .unwrap();
    assert!(!removed_replay.created);
    assert_eq!(removed_replay.source.state, KnowledgeSourceState::Removed);
    let successor_id = KnowledgeSourceId::new();
    let successor = repository
        .ensure_source(&EnsureKnowledgeSourceParams {
            knowledge_source_id: successor_id.clone(),
            knowledge_base_id,
            kind: KnowledgeSourceKind::Url,
            mode: KnowledgeSourceMode::Snapshot,
            default_parent_entry_id: None,
            created_at: 26,
        })
        .await
        .unwrap();
    assert!(successor.created);
    assert_eq!(successor.source.knowledge_source_id, successor_id);
}

#[tokio::test]
async fn remove_managed_source_item_is_atomic_and_replay_safe_from_both_live_and_detached_states() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&knowledge_base_id, "remove source"))
        .await
        .unwrap();
    let first_entry_id = KnowledgeEntryId::new();
    let second_entry_id = KnowledgeEntryId::new();
    repository
        .upsert_entry(&entry(
            &knowledge_base_id,
            &first_entry_id,
            "first.md",
            "file",
        ))
        .await
        .unwrap();
    repository
        .upsert_entry(&entry(
            &knowledge_base_id,
            &second_entry_id,
            "second.md",
            "file",
        ))
        .await
        .unwrap();
    let source_id = KnowledgeSourceId::new();
    repository
        .ensure_source(&source_params(&knowledge_base_id, &source_id, None))
        .await
        .unwrap();

    let first_item_id = KnowledgeSourceItemId::new();
    repository
        .create_source_item(&item_params(
            &source_id,
            &first_item_id,
            "https://example.test/first",
            0,
        ))
        .await
        .unwrap();
    repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: first_entry_id.clone(),
            knowledge_source_item_id: first_item_id.clone(),
            created_at: 20,
        })
        .await
        .unwrap();
    let attempted = repository
        .record_sync_attempt(&first_item_id, 0, 21)
        .await
        .unwrap();
    let staged = repository
        .stage_sync_publication(&StageKnowledgeSourcePublicationParams {
            knowledge_source_item_id: first_item_id.clone(),
            expected_revision: attempted.revision,
            pending_published_hash: "a".repeat(64),
            pending_final_url: None,
            pending_title: None,
            staged_at: 22,
        })
        .await
        .unwrap();
    let (detached, removed) = repository
        .remove_managed_source_item(&first_entry_id, 0, staged.revision, 23)
        .await
        .unwrap();
    assert_eq!(
        detached.relationship,
        KnowledgeEntryProvenanceRelationship::Detached
    );
    assert_eq!(detached.revision, 1);
    assert_eq!(removed.state, KnowledgeSourceState::Removed);
    assert_eq!(removed.revision, staged.revision + 1);
    assert_eq!(removed.sync_status, KnowledgeSourceItemSyncStatus::Missing);
    assert!(removed.pending_published_hash.is_none());
    let replay = repository
        .remove_managed_source_item(&first_entry_id, 0, staged.revision, 23)
        .await
        .unwrap();
    assert_eq!(replay, (detached, removed));

    let second_item_id = KnowledgeSourceItemId::new();
    repository
        .create_source_item(&item_params(
            &source_id,
            &second_item_id,
            "https://example.test/second",
            1,
        ))
        .await
        .unwrap();
    let managed = repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: second_entry_id.clone(),
            knowledge_source_item_id: second_item_id.clone(),
            created_at: 30,
        })
        .await
        .unwrap();
    let already_detached = repository
        .detach_managed_entry(&second_entry_id, managed.revision, 31)
        .await
        .unwrap();
    let paused = repository
        .get_source_item(&second_item_id)
        .await
        .unwrap()
        .unwrap();
    let (same_provenance, removed) = repository
        .remove_managed_source_item(
            &second_entry_id,
            already_detached.revision,
            paused.revision,
            32,
        )
        .await
        .unwrap();
    assert_eq!(same_provenance, already_detached);
    assert_eq!(removed.state, KnowledgeSourceState::Removed);
    assert_eq!(removed.revision, paused.revision + 1);
    let replay = repository
        .remove_managed_source_item(
            &second_entry_id,
            already_detached.revision,
            paused.revision,
            32,
        )
        .await
        .unwrap();
    assert_eq!(replay.0, already_detached);
    assert_eq!(replay.1, removed);
    validate_id_data_contract(database.pool()).await.unwrap();
}

#[tokio::test]
async fn provenance_rejects_cross_base_links_and_base_delete_cleans_the_whole_source_graph() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let first_base_id = KnowledgeBaseId::new();
    let second_base_id = KnowledgeBaseId::new();
    repository
        .insert_base(&base(&first_base_id, "first"))
        .await
        .unwrap();
    repository
        .insert_base(&base(&second_base_id, "second"))
        .await
        .unwrap();
    let first_entry_id = KnowledgeEntryId::new();
    let second_entry_id = KnowledgeEntryId::new();
    repository
        .upsert_entry(&entry(&first_base_id, &first_entry_id, "first.md", "file"))
        .await
        .unwrap();
    repository
        .upsert_entry(&entry(&second_base_id, &second_entry_id, "second.md", "file"))
        .await
        .unwrap();
    let source_id = KnowledgeSourceId::new();
    repository
        .ensure_source(&source_params(&first_base_id, &source_id, None))
        .await
        .unwrap();
    let item_id = KnowledgeSourceItemId::new();
    repository
        .create_source_item(&item_params(
            &source_id,
            &item_id,
            "https://example.test/page",
            0,
        ))
        .await
        .unwrap();

    let cross_base = repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: second_entry_id,
            knowledge_source_item_id: item_id.clone(),
            created_at: 20,
        })
        .await
        .unwrap_err();
    assert!(matches!(cross_base, DbError::Conflict(_)));
    repository
        .bind_managed_entry(&BindManagedKnowledgeEntryParams {
            knowledge_entry_id: first_entry_id,
            knowledge_source_item_id: item_id,
            created_at: 20,
        })
        .await
        .unwrap();

    repository.delete_base(first_base_id.as_str()).await.unwrap();
    for table in [
        "knowledge_entry_provenance",
        "knowledge_source_items",
        "knowledge_sources",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(count, 0, "{table} leaked after base deletion");
    }
    let remaining_entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_entries")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(remaining_entries, 1, "the unrelated base entry must survive");
    assert!(repository.get_base(second_base_id.as_str()).await.unwrap().is_some());
}
