use nomifun_common::{ConversationId, generate_prefixed_id};
use nomifun_db::backup_bundle::{
    BUNDLE_DATA_DIR, BUNDLE_WORK_DIR, BackupError, BackupObjectGraph, BackupSource,
    COMPANION_DIR, DATABASE_FILE, ENCRYPTION_KEY_FILE, ImportMode, MANAGED_WORKSPACES_DIR,
    MANIFEST_FILE,
    PortableCatalog, PortableEntity, PortableGraph, create_backup_bundle,
    create_backup_bundle_with_sources, restore_backup_bundle, verify_backup_bundle,
};
use nomifun_db::init_database;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

fn entity(
    entity_type: &str,
    prefix: &str,
    id: String,
    payload: serde_json::Value,
    references: impl IntoIterator<Item = (&'static str, String)>,
) -> PortableEntity {
    PortableEntity {
        entity_type: entity_type.to_owned(),
        id_prefix: prefix.to_owned(),
        id,
        payload,
        references: references
            .into_iter()
            .map(|(pointer, target)| (pointer.to_owned(), json!(target)))
            .collect(),
    }
}

fn conversation_graph() -> (PortableGraph, String, String) {
    let conversation_id = ConversationId::new().into_string();
    let message_id = generate_prefixed_id("msg");
    let graph = PortableGraph {
        entities: vec![
            entity(
                "conversation",
                "conv",
                conversation_id.clone(),
                json!({"name": "portable conversation"}),
                [],
            ),
            entity(
                "message",
                "msg",
                message_id.clone(),
                json!({
                    "conversation_id": conversation_id,
                    "content": {"text": "hello"}
                }),
                [("/conversation_id", conversation_id.clone())],
            ),
        ],
    };
    (graph, conversation_id, message_id)
}

#[test]
fn restore_and_merge_preserve_ids_and_are_idempotent() {
    let (graph, conversation_id, message_id) = conversation_graph();
    let mut catalog = PortableCatalog::default();

    let restored = catalog.import(&graph, ImportMode::Restore).unwrap();
    assert_eq!(restored.inserted, 2);
    assert_eq!(restored.skipped_identical, 0);
    assert!(restored.remap.is_empty());
    assert!(catalog.get(&conversation_id).is_some());
    assert_eq!(
        catalog.get(&message_id).unwrap().references["/conversation_id"],
        json!(conversation_id)
    );

    let merged = catalog.import(&graph, ImportMode::Merge).unwrap();
    assert_eq!(merged.inserted, 0);
    assert_eq!(merged.skipped_identical, 2);
    assert_eq!(catalog.len(), 2);
}

#[test]
fn restore_and_merge_reject_same_id_with_different_content_atomically() {
    let (graph, conversation_id, _) = conversation_graph();
    let mut catalog = PortableCatalog::default();
    catalog.import(&graph, ImportMode::Restore).unwrap();

    let mut conflicting = graph.clone();
    conflicting.entities[0].payload = json!({"name": "different content"});
    let before = catalog.clone();
    let error = catalog
        .import(&conflicting, ImportMode::Merge)
        .expect_err("same ID with divergent content must fail");
    assert!(matches!(
        error,
        BackupError::Conflict { id, .. } if id == conversation_id
    ));
    assert_eq!(catalog, before, "conflicting merge must be all-or-nothing");
}

#[test]
fn clone_mints_new_ids_and_rewrites_every_internal_reference() {
    let (graph, old_conversation_id, old_message_id) = conversation_graph();
    let mut catalog = PortableCatalog::default();
    catalog.import(&graph, ImportMode::Restore).unwrap();

    let cloned = catalog.import(&graph, ImportMode::Clone).unwrap();
    assert_eq!(cloned.inserted, 2);
    assert_eq!(cloned.remap.len(), 2);
    let new_conversation_id = &cloned.remap[&old_conversation_id];
    let new_message_id = &cloned.remap[&old_message_id];
    assert_ne!(new_conversation_id, &old_conversation_id);
    assert_ne!(new_message_id, &old_message_id);
    assert!(new_conversation_id.starts_with("conv_"));
    assert!(new_message_id.starts_with("msg_"));
    assert_eq!(
        catalog.get(new_message_id).unwrap().references["/conversation_id"],
        json!(new_conversation_id)
    );
    assert_eq!(
        catalog.get(new_message_id).unwrap().payload["conversation_id"],
        json!(new_conversation_id)
    );
    assert_eq!(catalog.len(), 4);
}

#[test]
fn clone_recursively_rewrites_declared_arrays_and_nested_reference_objects() {
    let conversation_id = ConversationId::new().into_string();
    let first_message_id = generate_prefixed_id("msg");
    let second_message_id = generate_prefixed_id("msg");
    let conversation_payload = json!({
        "lead_message_id": first_message_id,
        "message_ids": [first_message_id, second_message_id],
        "relations": {
            "lead": first_message_id,
            "alternates": [second_message_id]
        }
    });
    let graph = PortableGraph {
        entities: vec![
            PortableEntity {
                entity_type: "conversation".into(),
                id_prefix: "conv".into(),
                id: conversation_id.clone(),
                payload: conversation_payload.clone(),
                references: [
                    (
                        "/lead_message_id".into(),
                        conversation_payload["lead_message_id"].clone(),
                    ),
                    (
                        "/message_ids".into(),
                        conversation_payload["message_ids"].clone(),
                    ),
                    (
                        "/relations".into(),
                        conversation_payload["relations"].clone(),
                    ),
                ]
                .into_iter()
                .collect(),
            },
            entity(
                "message",
                "msg",
                first_message_id.clone(),
                json!({"conversation_id": conversation_id}),
                [("/conversation_id", conversation_id.clone())],
            ),
            entity(
                "message",
                "msg",
                second_message_id.clone(),
                json!({"conversation_id": conversation_id}),
                [("/conversation_id", conversation_id.clone())],
            ),
        ],
    };

    let mut catalog = PortableCatalog::default();
    let cloned = catalog.import(&graph, ImportMode::Clone).unwrap();
    let new_conversation_id = &cloned.remap[&conversation_id];
    let new_first_message_id = &cloned.remap[&first_message_id];
    let new_second_message_id = &cloned.remap[&second_message_id];
    let cloned_conversation = catalog.get(new_conversation_id).unwrap();
    assert_eq!(
        cloned_conversation.payload["message_ids"],
        json!([new_first_message_id, new_second_message_id])
    );
    assert_eq!(
        cloned_conversation.payload["relations"],
        json!({
            "lead": new_first_message_id,
            "alternates": [new_second_message_id]
        })
    );
    for new_message_id in [new_first_message_id, new_second_message_id] {
        assert_eq!(
            catalog.get(new_message_id).unwrap().payload["conversation_id"],
            json!(new_conversation_id)
        );
    }
}

#[test]
fn clone_rejects_undeclared_or_mismatched_reference_pointer_atomically() {
    let (mut graph, conversation_id, message_id) = conversation_graph();
    graph.entities[1]
        .references
        .insert("/missing".into(), json!(conversation_id));
    let mut catalog = PortableCatalog::default();
    let before = catalog.clone();
    let error = catalog
        .import(&graph, ImportMode::Clone)
        .expect_err("missing declared reference pointer must fail");
    assert!(matches!(error, BackupError::InvalidGraph(_)));
    assert_eq!(catalog, before);
    assert!(catalog.get(&message_id).is_none());
}

#[tokio::test]
async fn bundle_manifest_captures_generation_graph_checksum_and_wal_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&source).await.unwrap();
    sqlx::query("CREATE TABLE bundle_probe (value TEXT PRIMARY KEY)")
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO bundle_probe(value) VALUES ('committed-in-wal')")
        .execute(database.pool())
        .await
        .unwrap();

    let generation = uuid::Uuid::now_v7().to_string();
    let manifest = create_backup_bundle(
        &database,
        &bundle,
        &generation,
        BackupObjectGraph {
            roots: vec![ConversationId::new().into_string()],
            entity_kinds: vec!["conversation".into(), "message".into()],
        },
    )
    .await
    .unwrap();
    assert_eq!(manifest.source_storage_generation, generation);
    assert_eq!(manifest.files.len(), 1);
    assert_eq!(manifest.files[0].path, DATABASE_FILE);
    assert_eq!(manifest.files[0].sha256.len(), 64);
    assert!(manifest.created_at > 0);
    assert_eq!(
        verify_backup_bundle(&bundle).unwrap(),
        manifest,
        "manifest must round-trip and verify"
    );

    let snapshot = open_read_only_pool(&bundle.join(DATABASE_FILE)).await;
    let value: String = sqlx::query_scalar("SELECT value FROM bundle_probe")
        .fetch_one(&snapshot)
        .await
        .unwrap();
    assert_eq!(value, "committed-in-wal");
    snapshot.close().await;

    let bytes = std::fs::read(bundle.join(DATABASE_FILE)).unwrap();
    assert_eq!(
        manifest.files[0].sha256,
        hex::encode(Sha256::digest(bytes))
    );
}

#[tokio::test]
async fn complete_bundle_captures_key_companion_and_managed_workspaces_only() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work_dir = dir.path().join("custom-work");
    let source = data_dir.join("nomifun-backend.db");
    let bundle = dir.path().join("backup.nomifun");
    std::fs::create_dir_all(&data_dir).unwrap();
    let database = init_database(&source).await.unwrap();

    std::fs::write(data_dir.join(ENCRYPTION_KEY_FILE), "ab".repeat(32)).unwrap();
    std::fs::create_dir_all(data_dir.join(COMPANION_DIR).join("shared")).unwrap();
    std::fs::create_dir_all(data_dir.join(COMPANION_DIR).join("empty-profile")).unwrap();
    std::fs::write(
        data_dir.join(COMPANION_DIR).join("shared/config.json"),
        br#"{"enabled":true}"#,
    )
    .unwrap();
    std::fs::create_dir_all(work_dir.join(MANAGED_WORKSPACES_DIR).join("empty-workspace"))
        .unwrap();
    std::fs::create_dir_all(
        work_dir
            .join(MANAGED_WORKSPACES_DIR)
            .join("nomi-temp-ws_test"),
    )
    .unwrap();
    std::fs::write(
        work_dir
            .join(MANAGED_WORKSPACES_DIR)
            .join("nomi-temp-ws_test/output.txt"),
        b"managed workspace",
    )
    .unwrap();
    std::fs::create_dir_all(data_dir.join("logs")).unwrap();
    std::fs::write(data_dir.join("logs/backend.log"), b"not portable").unwrap();
    std::fs::create_dir_all(data_dir.join("bun-cache")).unwrap();
    std::fs::write(data_dir.join("bun-cache/runtime.bin"), b"cache").unwrap();
    let custom_external_workspace = dir.path().join("user-project");
    std::fs::create_dir_all(&custom_external_workspace).unwrap();
    std::fs::write(custom_external_workspace.join("private.txt"), b"external").unwrap();
    for excluded in ["logs", "cache", "custom-user-project"] {
        std::fs::create_dir_all(work_dir.join(excluded)).unwrap();
        std::fs::write(work_dir.join(excluded).join("excluded.txt"), b"excluded").unwrap();
    }

    let manifest = create_backup_bundle_with_sources(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
        BackupSource::new(&data_dir, &work_dir),
    )
    .await
    .unwrap();
    let paths: std::collections::BTreeSet<String> =
        manifest.files.iter().map(|file| file.path.clone()).collect();
    assert!(paths.contains(DATABASE_FILE));
    assert!(paths.contains(&format!("{BUNDLE_DATA_DIR}/{ENCRYPTION_KEY_FILE}")));
    assert!(paths.contains(&format!(
        "{BUNDLE_DATA_DIR}/{COMPANION_DIR}/shared/config.json"
    )));
    assert!(paths.contains(&format!(
        "{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}/nomi-temp-ws_test/output.txt"
    )));
    assert!(paths.iter().all(|path| !path.contains("logs")));
    assert!(paths.iter().all(|path| !path.contains("bun-cache")));
    assert!(paths.iter().all(|path| !path.contains("user-project")));
    assert!(paths.iter().all(|path| !path.contains("custom-user-project")));
    assert!(paths.iter().all(|path| !path.contains("excluded.txt")));
    assert!(manifest.directories.contains(&format!(
        "{BUNDLE_DATA_DIR}/{COMPANION_DIR}/empty-profile"
    )));
    assert!(manifest.directories.contains(&format!(
        "{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}/empty-workspace"
    )));
    assert!(!manifest.layout.custom_external_workspaces_included);
    assert_eq!(verify_backup_bundle(&bundle).unwrap(), manifest);
}

#[tokio::test]
async fn bundle_verification_fails_closed_after_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&source).await.unwrap();
    create_backup_bundle(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();

    std::fs::write(bundle.join(DATABASE_FILE), b"tampered").unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::ChecksumMismatch { .. })
    ));
}

#[tokio::test]
async fn offline_restore_preserves_entity_ids_and_rotates_dataset_generation() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let restored_database = dir.path().join("restored").join("nomifun-backend.db");
    let restored_generation = dir.path().join("restored").join("storage-generation");
    let database = init_database(&source).await.unwrap();
    let source_owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
    let conversation_id = ConversationId::new().into_string();
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, created_at, updated_at) \
         VALUES (?, ?, 'preserved', 'nomi', '{}', 'pending', 1, 1)",
    )
    .bind(&conversation_id)
    .bind(&source_owner)
    .execute(database.pool())
    .await
    .unwrap();
    let source_generation = uuid::Uuid::now_v7().to_string();
    create_backup_bundle(
        &database,
        &bundle,
        &source_generation,
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();

    let outcome = restore_backup_bundle(&bundle, &restored_database, &restored_generation)
        .await
        .unwrap();
    assert_eq!(
        outcome.manifest.source_storage_generation,
        source_generation
    );
    assert_ne!(
        outcome.destination_storage_generation,
        source_generation,
        "a restore is a new dataset namespace, even though entity IDs survive"
    );
    assert_eq!(
        std::fs::read_to_string(&restored_generation).unwrap(),
        outcome.destination_storage_generation
    );

    let restored = open_read_only_pool(&restored_database).await;
    let restored_id: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE name = 'preserved'")
            .fetch_one(&restored)
            .await
            .unwrap();
    let restored_owner: String = sqlx::query_scalar(
        "SELECT owner_user_id FROM installation_identity WHERE key = 'installation'",
    )
    .fetch_one(&restored)
    .await
    .unwrap();
    assert_eq!(restored_id, conversation_id);
    assert_eq!(restored_owner, source_owner);
    restored.close().await;

    assert!(
        restore_backup_bundle(&bundle, &restored_database, &restored_generation)
            .await
            .is_err(),
        "offline restore must never overwrite an existing dataset"
    );
}

#[tokio::test]
async fn complete_restore_is_atomic_and_materializes_all_portable_domains() {
    let dir = tempfile::tempdir().unwrap();
    let source_root = dir.path().join("source");
    let work_root = dir.path().join("work");
    let source_database = source_root.join("nomifun-backend.db");
    let bundle = dir.path().join("backup.nomifun");
    let destination = dir.path().join("restored");
    std::fs::create_dir_all(&source_root).unwrap();
    let database = init_database(&source_database).await.unwrap();
    std::fs::write(source_root.join(ENCRYPTION_KEY_FILE), "cd".repeat(32)).unwrap();
    std::fs::create_dir_all(source_root.join(COMPANION_DIR).join("shared")).unwrap();
    std::fs::create_dir_all(source_root.join(COMPANION_DIR).join("empty")).unwrap();
    std::fs::write(
        source_root
            .join(COMPANION_DIR)
            .join("shared/memory-export.json"),
        b"memories",
    )
    .unwrap();
    std::fs::create_dir_all(work_root.join(MANAGED_WORKSPACES_DIR).join("managed")).unwrap();
    std::fs::create_dir_all(work_root.join(MANAGED_WORKSPACES_DIR).join("empty")).unwrap();
    std::fs::write(
        work_root
            .join(MANAGED_WORKSPACES_DIR)
            .join("managed/file.md"),
        b"result",
    )
    .unwrap();
    create_backup_bundle_with_sources(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
        BackupSource::new(&source_root, &work_root),
    )
    .await
    .unwrap();
    database.close().await;

    let outcome = restore_backup_bundle(
        &bundle,
        &destination.join("nomifun-backend.db"),
        &destination.join("storage-generation"),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(destination.join(ENCRYPTION_KEY_FILE)).unwrap(),
        "cd".repeat(32)
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("companion/shared/memory-export.json"))
            .unwrap(),
        "memories"
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("conversations/managed/file.md")).unwrap(),
        "result"
    );
    assert!(destination.join("companion/empty").is_dir());
    assert!(destination.join("conversations/empty").is_dir());
    assert_eq!(
        std::fs::read_to_string(destination.join("storage-generation")).unwrap(),
        outcome.destination_storage_generation
    );

    let corrupt_bundle = dir.path().join("corrupt.nomifun");
    copy_tree_for_test(&bundle, &corrupt_bundle);
    std::fs::write(corrupt_bundle.join(DATABASE_FILE), b"corrupt").unwrap();
    let untouched = dir.path().join("untouched");
    assert!(
        restore_backup_bundle(
            &corrupt_bundle,
            &untouched.join("nomifun-backend.db"),
            &untouched.join("storage-generation"),
        )
        .await
        .is_err()
    );
    assert!(!untouched.exists(), "failed restore must not expose a partial target");
}

#[tokio::test]
async fn restore_rejects_valid_sqlite_with_wrong_schema_after_checksum_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&source).await.unwrap();
    create_backup_bundle(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();
    database.close().await;

    let wrong_database = dir.path().join("wrong-schema.db");
    let wrong_pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&wrong_database)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE unrelated (id INTEGER PRIMARY KEY)")
        .execute(&wrong_pool)
        .await
        .unwrap();
    wrong_pool.close().await;
    std::fs::copy(&wrong_database, bundle.join(DATABASE_FILE)).unwrap();
    rewrite_database_manifest_entry(&bundle);
    verify_backup_bundle(&bundle).expect("file-level checksums should now be internally valid");

    let destination = dir.path().join("must-stay-absent");
    let error = restore_backup_bundle(
        &bundle,
        &destination.join("nomifun-backend.db"),
        &destination.join("storage-generation"),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error}").contains("ID contract"),
        "unexpected validation failure: {error}"
    );
    assert!(!destination.exists());
}

#[tokio::test]
async fn restore_rejects_missing_installation_identity_after_checksum_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&dir.path().join("source.db")).await.unwrap();
    create_backup_bundle(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();
    database.close().await;

    let bundle_pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(bundle.join(DATABASE_FILE))
            .create_if_missing(false),
    )
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER installation_identity_delete_guard")
        .execute(&bundle_pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM installation_identity")
        .execute(&bundle_pool)
        .await
        .unwrap();
    bundle_pool.close().await;
    rewrite_database_manifest_entry(&bundle);
    verify_backup_bundle(&bundle).unwrap();

    let destination = dir.path().join("must-stay-absent");
    let error = restore_backup_bundle(
        &bundle,
        &destination.join("nomifun-backend.db"),
        &destination.join("storage-generation"),
    )
    .await
    .unwrap_err();
    assert!(format!("{error}").contains("exactly one row"));
    assert!(!destination.exists());
}

#[tokio::test]
async fn restore_rejects_noncanonical_row_ids_after_checksum_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&dir.path().join("source.db")).await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, created_at, updated_at) \
         VALUES (?, ?, 'canonical probe', 'nomi', '{}', 'pending', 1, 1)",
    )
    .bind(ConversationId::new().into_string())
    .bind(owner)
    .execute(database.pool())
    .await
    .unwrap();
    create_backup_bundle(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();
    database.close().await;

    let bundle_pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(bundle.join(DATABASE_FILE))
            .create_if_missing(false),
    )
    .await
    .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&bundle_pool)
        .await
        .unwrap();
    sqlx::query("UPDATE conversations SET id = 'conv_bad'")
        .execute(&bundle_pool)
        .await
        .unwrap();
    bundle_pool.close().await;
    rewrite_database_manifest_entry(&bundle);

    let destination = dir.path().join("must-stay-absent");
    let error = restore_backup_bundle(
        &bundle,
        &destination.join("nomifun-backend.db"),
        &destination.join("storage-generation"),
    )
    .await
    .unwrap_err();
    assert!(format!("{error}").contains("not canonical"));
    assert!(!destination.exists());
}

#[tokio::test]
async fn traversal_and_undeclared_payloads_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&source).await.unwrap();
    create_backup_bundle(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();

    std::fs::write(bundle.join("undeclared"), b"x").unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::InvalidManifest(_))
    ));
    std::fs::remove_file(bundle.join("undeclared")).unwrap();

    let manifest_path = bundle.join(MANIFEST_FILE);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["files"][0]["path"] = json!("../database.sqlite3");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::InvalidManifest(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_sources_and_broken_link_destinations_fail_closed_without_staging_debris() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work_dir = dir.path().join("work");
    let bundle = dir.path().join("backup.nomifun");
    std::fs::create_dir_all(data_dir.join(COMPANION_DIR)).unwrap();
    std::fs::create_dir_all(&work_dir).unwrap();
    let database = init_database(&data_dir.join("nomifun-backend.db"))
        .await
        .unwrap();
    let external = dir.path().join("external-secret");
    std::fs::write(&external, b"secret").unwrap();
    symlink(
        &external,
        data_dir.join(COMPANION_DIR).join("linked-secret"),
    )
    .unwrap();

    let error = create_backup_bundle_with_sources(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
        BackupSource::new(&data_dir, &work_dir),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BackupError::UnsafeSource(_)));
    assert!(!bundle.exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".backup.nomifun.staging-")
    }));

    std::fs::remove_file(data_dir.join(COMPANION_DIR).join("linked-secret")).unwrap();
    create_backup_bundle_with_sources(
        &database,
        &bundle,
        &uuid::Uuid::now_v7().to_string(),
        BackupObjectGraph::full_database(),
        BackupSource::new(&data_dir, &work_dir),
    )
    .await
    .unwrap();
    database.close().await;

    let broken_target = dir.path().join("missing-target");
    let destination = dir.path().join("restore-link");
    symlink(&broken_target, &destination).unwrap();
    assert!(
        restore_backup_bundle(
            &bundle,
            &destination.join("nomifun-backend.db"),
            &destination.join("storage-generation"),
        )
        .await
        .is_err()
    );
    assert!(
        std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

fn rewrite_database_manifest_entry(bundle: &std::path::Path) {
    let manifest_path = bundle.join(MANIFEST_FILE);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let database_path = bundle.join(DATABASE_FILE);
    let bytes = std::fs::read(&database_path).unwrap();
    let entry = manifest["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["path"] == DATABASE_FILE)
        .unwrap();
    entry["bytes"] = json!(bytes.len() as u64);
    entry["sha256"] = json!(hex::encode(Sha256::digest(bytes)));
    std::fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn copy_tree_for_test(source: &std::path::Path, destination: &std::path::Path) {
    std::fs::create_dir(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree_for_test(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

async fn open_read_only_pool(path: &std::path::Path) -> SqlitePool {
    use std::str::FromStr;

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .read_only(true)
        .create_if_missing(false);
    SqlitePool::connect_with(options).await.unwrap()
}
