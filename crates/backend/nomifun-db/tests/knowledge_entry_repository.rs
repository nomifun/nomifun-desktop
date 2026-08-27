use nomifun_common::{KnowledgeBaseId, KnowledgeEntryId};
use nomifun_db::{
    DbError, IKnowledgeEntryRepository, IKnowledgeRepository, KnowledgeBaseRow,
    RelocateKnowledgeEntryProjectionParams, SqliteKnowledgeRepository,
    UpsertKnowledgeEntryParams, init_database_memory,
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

fn base(knowledge_base_id: &KnowledgeBaseId) -> KnowledgeBaseRow {
    KnowledgeBaseRow {
        id: 0,
        knowledge_base_id: knowledge_base_id.to_string(),
        name: "projection fixture".into(),
        description: String::new(),
        root_path: format!("/tmp/{knowledge_base_id}"),
        managed: true,
        extra: "{}".into(),
        created_at: 1,
        updated_at: 1,
        tags: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn entry(
    knowledge_base_id: &KnowledgeBaseId,
    knowledge_entry_id: &KnowledgeEntryId,
    parent_entry_id: Option<&KnowledgeEntryId>,
    name: &str,
    kind: &str,
    rel_path: &str,
    portable_rel_path: &str,
    revision: i64,
) -> UpsertKnowledgeEntryParams {
    UpsertKnowledgeEntryParams {
        knowledge_entry_id: knowledge_entry_id.clone(),
        knowledge_base_id: knowledge_base_id.clone(),
        parent_entry_id: parent_entry_id.cloned(),
        name: name.into(),
        kind: kind.into(),
        origin: "user".into(),
        rel_path: rel_path.into(),
        portable_rel_path: portable_rel_path.into(),
        fs_identity: Some(format!("identity:{knowledge_entry_id}")),
        content_hash: (kind == "file").then(|| format!("hash:{knowledge_entry_id}")),
        revision,
        deleted_at: None,
        created_at: 10,
        updated_at: 10,
    }
}

#[tokio::test]
async fn migration_adds_rebuildable_projection_without_rewriting_existing_bases() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&pool, 52).await;
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
    let projection_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'table' AND name = 'knowledge_entries'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(projection_before, 0);

    migrate_through(&pool, 53).await;

    let preserved: (String, i64) = sqlx::query_as(
        "SELECT name, tree_revision FROM knowledge_bases WHERE knowledge_base_id = ?",
    )
    .bind(knowledge_base_id.as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(preserved, ("existing".into(), 0));
    let projection_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'table' AND name = 'knowledge_entries'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(projection_after, 1);
}

#[tokio::test]
async fn replace_and_relocate_directory_preserve_every_identity_atomically() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository.insert_base(&base(&knowledge_base_id)).await.unwrap();

    let docs_id = KnowledgeEntryId::new();
    let archive_id = KnowledgeEntryId::new();
    let notes_id = KnowledgeEntryId::new();
    let note_id = KnowledgeEntryId::new();
    let snapshot = vec![
        entry(
            &knowledge_base_id,
            &docs_id,
            None,
            "Docs",
            "directory",
            "Docs",
            "docs",
            0,
        ),
        entry(
            &knowledge_base_id,
            &archive_id,
            None,
            "Archive",
            "directory",
            "Archive",
            "archive",
            0,
        ),
        entry(
            &knowledge_base_id,
            &notes_id,
            Some(&docs_id),
            "Notes",
            "directory",
            "Docs/Notes",
            "docs/notes",
            0,
        ),
        entry(
            &knowledge_base_id,
            &note_id,
            Some(&notes_id),
            "A.md",
            "file",
            "Docs/Notes/A.md",
            "docs/notes/a.md",
            4,
        ),
    ];

    let replacement = repository
        .replace_projection(&knowledge_base_id, Some(0), &snapshot)
        .await
        .unwrap();
    assert_eq!(replacement.replaced_entries, 4);
    assert_eq!(replacement.tree_revision, 1);
    assert_eq!(
        repository
            .get_entry_by_path(&knowledge_base_id, "docs/notes/a.md")
            .await
            .unwrap()
            .unwrap()
            .knowledge_entry_id,
        note_id
    );

    let moved = repository
        .relocate_entry(&RelocateKnowledgeEntryProjectionParams {
            knowledge_base_id: knowledge_base_id.clone(),
            knowledge_entry_id: notes_id.clone(),
            destination_parent_entry_id: Some(archive_id.clone()),
            new_name: "Notes".into(),
            new_rel_path: "Archive/Notes".into(),
            new_portable_rel_path: "archive/notes".into(),
            expected_revision: 0,
            updated_at: 20,
        })
        .await
        .unwrap();
    assert_eq!(moved.affected_entries, 2, "directory plus markdown child");
    assert_eq!(moved.tree_revision, 2);
    assert_eq!(moved.entry.knowledge_entry_id, notes_id);
    assert_eq!(moved.entry.parent_entry_id.as_ref(), Some(&archive_id));
    assert_eq!(moved.entry.revision, 1);

    let child = repository
        .get_entry(&knowledge_base_id, &note_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.knowledge_entry_id, note_id);
    assert_eq!(child.parent_entry_id.as_ref(), Some(&notes_id));
    assert_eq!(child.rel_path, "Archive/Notes/A.md");
    assert_eq!(child.portable_rel_path, "archive/notes/a.md");
    assert_eq!(child.revision, 5);
    assert!(
        repository
            .get_entry_by_path(&knowledge_base_id, "docs/notes/a.md")
            .await
            .unwrap()
            .is_none()
    );

    let no_op = repository
        .relocate_entry(&RelocateKnowledgeEntryProjectionParams {
            knowledge_base_id: knowledge_base_id.clone(),
            knowledge_entry_id: notes_id.clone(),
            destination_parent_entry_id: Some(archive_id.clone()),
            new_name: "Notes".into(),
            new_rel_path: "Archive/Notes".into(),
            new_portable_rel_path: "archive/notes".into(),
            expected_revision: 1,
            updated_at: 30,
        })
        .await
        .unwrap();
    assert_eq!(no_op.affected_entries, 0);
    assert_eq!(no_op.tree_revision, 2, "a no-op does not create an event revision");

    let cycle = repository
        .relocate_entry(&RelocateKnowledgeEntryProjectionParams {
            knowledge_base_id: knowledge_base_id.clone(),
            knowledge_entry_id: archive_id.clone(),
            destination_parent_entry_id: Some(notes_id.clone()),
            new_name: "Archive".into(),
            new_rel_path: "Archive/Notes/Archive".into(),
            new_portable_rel_path: "archive/notes/archive".into(),
            expected_revision: 0,
            updated_at: 30,
        })
        .await;
    assert!(matches!(cycle, Err(DbError::Conflict(_))));
    assert_eq!(repository.tree_revision(&knowledge_base_id).await.unwrap(), 2);
    nomifun_db::validate_id_data_contract(database.pool())
        .await
        .expect("moved projection must satisfy logical parent/base relationships");
}

#[tokio::test]
async fn replace_is_cas_and_invalid_snapshots_roll_back_without_partial_rows() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository.insert_base(&base(&knowledge_base_id)).await.unwrap();
    let root_id = KnowledgeEntryId::new();
    let original = vec![entry(
        &knowledge_base_id,
        &root_id,
        None,
        "Root",
        "directory",
        "Root",
        "root",
        0,
    )];
    repository
        .replace_projection(&knowledge_base_id, Some(0), &original)
        .await
        .unwrap();

    let stale = repository
        .replace_projection(&knowledge_base_id, Some(0), &[])
        .await;
    assert!(matches!(stale, Err(DbError::Conflict(_))));

    let orphan_id = KnowledgeEntryId::new();
    let missing_parent_id = KnowledgeEntryId::new();
    let invalid = vec![entry(
        &knowledge_base_id,
        &orphan_id,
        Some(&missing_parent_id),
        "orphan.md",
        "file",
        "missing/orphan.md",
        "missing/orphan.md",
        0,
    )];
    let invalid_result = repository
        .replace_projection(&knowledge_base_id, Some(1), &invalid)
        .await;
    assert!(matches!(invalid_result, Err(DbError::Conflict(_))));

    let rows = repository
        .list_entries_for_base(&knowledge_base_id, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].knowledge_entry_id, root_id);
    assert_eq!(repository.tree_revision(&knowledge_base_id).await.unwrap(), 1);
}

#[tokio::test]
async fn soft_delete_keeps_tombstones_releases_path_and_base_delete_cleans_projection() {
    let database = init_database_memory().await.unwrap();
    let repository = SqliteKnowledgeRepository::new(database.pool().clone());
    let knowledge_base_id = KnowledgeBaseId::new();
    repository.insert_base(&base(&knowledge_base_id)).await.unwrap();
    let directory_id = KnowledgeEntryId::new();
    let file_id = KnowledgeEntryId::new();
    repository
        .replace_projection(
            &knowledge_base_id,
            Some(0),
            &[
                entry(
                    &knowledge_base_id,
                    &directory_id,
                    None,
                    "Notes",
                    "directory",
                    "Notes",
                    "notes",
                    0,
                ),
                entry(
                    &knowledge_base_id,
                    &file_id,
                    Some(&directory_id),
                    "a.md",
                    "file",
                    "Notes/a.md",
                    "notes/a.md",
                    0,
                ),
            ],
        )
        .await
        .unwrap();

    let deleted = repository
        .soft_delete_entry_subtree(&knowledge_base_id, &directory_id, 0, 50)
        .await
        .unwrap();
    assert_eq!(deleted.affected_entries, 2);
    assert!(deleted.entry.is_deleted());
    assert!(
        repository
            .list_entries_for_base(&knowledge_base_id, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repository
            .list_entries_for_base(&knowledge_base_id, true)
            .await
            .unwrap()
            .len(),
        2
    );

    let replacement_id = KnowledgeEntryId::new();
    repository
        .upsert_entry(&entry(
            &knowledge_base_id,
            &replacement_id,
            None,
            "Notes",
            "directory",
            "Notes",
            "notes",
            0,
        ))
        .await
        .expect("a tombstone must not reserve its former path forever");
    assert_eq!(
        repository
            .get_entry_by_path(&knowledge_base_id, "notes")
            .await
            .unwrap()
            .unwrap()
            .knowledge_entry_id,
        replacement_id
    );

    repository.delete_base(knowledge_base_id.as_str()).await.unwrap();
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_entries WHERE knowledge_base_id = ?",
    )
    .bind(knowledge_base_id.as_str())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(remaining, 0, "logical cascade is repository-coordinated");
}
