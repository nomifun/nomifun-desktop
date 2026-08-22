use nomifun_common::{
    ConversationId,
    dataset_roots::{
        BackupPolicy, DatasetRootKind, WORK_ROOT_BINDING_FILE,
        WORK_ROOT_OWNER_FILE, managed_dataset_roots,
    },
    factory_reset::{DatasetPreparation, prepare_v3_dataset},
    generate_id,
};
use nomifun_db::backup_bundle::{
    BACKUP_FORMAT_VERSION, BACKUP_SCHEMA, BUNDLE_DATA_DIR, BUNDLE_WORK_DIR, BackupCoverageKind,
    BackupCoverageRoot, BackupError, BackupObjectGraph, BackupSource, COMPANION_DIR,
    DATABASE_FILE, DATASET_RECEIPT_FILE, ENCRYPTION_KEY_FILE,
    MANAGED_WORKSPACES_DIR, MANIFEST_FILE, STORAGE_GENERATION_FILE,
    create_backup_bundle,
    create_backup_bundle_with_sources, restore_backup_data_dir, verify_backup_bundle,
};
use nomifun_db::init_database;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

const REQUIRED_PORTABLE_DIRECTORY_ROOTS: &[&str] = &[
    "attachments",
    "knowledge",
    "projects",
    "workshop",
    "public-agents",
    "browser-state",
    "skills",
    "preset-rules",
    "preset-skills",
    "preset-instructions",
    "preset-avatars",
    "extensions",
];

#[tokio::test]
async fn bundle_manifest_captures_generation_graph_checksum_and_wal_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&source).await.unwrap();
    sqlx::query(
        "INSERT INTO client_preferences(key, value, updated_at) \
         VALUES ('bundle-probe', 'committed-in-wal', 1)",
    )
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
    assert_eq!(manifest.schema, BACKUP_SCHEMA);
    assert_eq!(manifest.schema, "id-contract-v3");
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
    let value: String =
        sqlx::query_scalar("SELECT value FROM client_preferences WHERE key = 'bundle-probe'")
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
async fn bundle_verification_rejects_v2_manifest_and_prefixed_graph_root() {
    let dir = tempfile::tempdir().unwrap();
    let database = init_database(&dir.path().join("source.db")).await.unwrap();

    let prefixed_bundle = dir.path().join("prefixed.nomifun");
    let bare_root = generate_id();
    let error = create_backup_bundle(
        &database,
        &prefixed_bundle,
        &generate_id(),
        BackupObjectGraph {
            roots: vec![format!("conv_{bare_root}")],
            entity_kinds: vec!["conversation".into()],
        },
    )
    .await
    .expect_err("v3 manifests must reject prefixed graph roots");
    assert!(matches!(error, BackupError::InvalidManifest(_)));
    assert!(!prefixed_bundle.exists());

    let bundle = dir.path().join("v2.nomifun");
    create_backup_bundle(
        &database,
        &bundle,
        &generate_id(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();
    let manifest_path = bundle.join(MANIFEST_FILE);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["schema"] = json!("id-contract-v2");
    std::fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let error = verify_backup_bundle(&bundle)
        .expect_err("v3 verification must reject v2 manifests without migration");
    assert!(matches!(error, BackupError::InvalidManifest(_)));
}

#[tokio::test]
async fn complete_bundle_captures_every_included_root_and_records_all_exclusions() {
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
    for root in managed_dataset_roots() {
        let path = data_dir.join(root.path);
        match (root.backup, root.kind) {
            (BackupPolicy::Include, DatasetRootKind::File) => {
                if root.path != ENCRYPTION_KEY_FILE {
                    std::fs::write(&path, format!("included file {}", root.path)).unwrap();
                }
            }
            (BackupPolicy::Include, DatasetRootKind::Directory) => {
                std::fs::create_dir_all(path.join("empty")).unwrap();
                std::fs::write(
                    path.join("payload.txt"),
                    format!("included directory {}", root.path),
                )
                .unwrap();
            }
            (BackupPolicy::Exclude(_), DatasetRootKind::File) => {
                std::fs::write(&path, format!("excluded file {}", root.path)).unwrap();
            }
            (BackupPolicy::Exclude(_), DatasetRootKind::Directory) => {
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("excluded.txt"), b"excluded").unwrap();
            }
        }
    }
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
    assert_eq!(manifest.format_version, BACKUP_FORMAT_VERSION);
    assert_eq!(manifest.format_version, 2);
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
    for root in REQUIRED_PORTABLE_DIRECTORY_ROOTS {
        assert!(
            paths.contains(&format!("{BUNDLE_DATA_DIR}/{root}/payload.txt")),
            "required portable root {root} was not backed up"
        );
        assert!(
            manifest
                .directories
                .contains(&format!("{BUNDLE_DATA_DIR}/{root}/empty")),
            "empty directory in required portable root {root} was not preserved"
        );
    }
    for root in managed_dataset_roots() {
        let coverage = match root.backup {
            BackupPolicy::Include => manifest.coverage.included.iter().find(|entry| {
                entry.root == BackupCoverageRoot::DataDir && entry.path == root.path
            }),
            BackupPolicy::Exclude(_) => manifest.coverage.excluded.iter().find(|entry| {
                entry.root == BackupCoverageRoot::DataDir && entry.path == root.path
            }),
        }
        .unwrap_or_else(|| panic!("coverage is missing {}", root.path));
        assert_eq!(
            coverage.kind,
            match root.kind {
                DatasetRootKind::File => BackupCoverageKind::File,
                DatasetRootKind::Directory => BackupCoverageKind::Directory,
            }
        );
        match root.backup {
            BackupPolicy::Include => {
                assert!(coverage.included);
                assert_eq!(coverage.exclusion_reason, None);
                match root.kind {
                    DatasetRootKind::File => {
                        assert!(paths.contains(&format!("{BUNDLE_DATA_DIR}/{}", root.path)));
                    }
                    DatasetRootKind::Directory => {
                        assert!(paths.contains(&format!(
                            "{BUNDLE_DATA_DIR}/{}/payload.txt",
                            root.path
                        )));
                        assert!(manifest.directories.contains(&format!(
                            "{BUNDLE_DATA_DIR}/{}/empty",
                            root.path
                        )));
                    }
                }
            }
            BackupPolicy::Exclude(reason) => {
                assert!(!coverage.included);
                assert_eq!(coverage.exclusion_reason.as_deref(), Some(reason));
                let bundle_root = format!("{BUNDLE_DATA_DIR}/{}", root.path);
                assert!(
                    paths
                        .iter()
                        .all(|path| path != &bundle_root
                            && !path.starts_with(&format!("{bundle_root}/"))),
                    "excluded root {} leaked into the bundle",
                    root.path
                );
                assert!(
                    manifest
                        .directories
                        .iter()
                        .all(|path| path != &bundle_root
                            && !path.starts_with(&format!("{bundle_root}/"))),
                    "excluded root {} leaked into the directory manifest",
                    root.path
                );
                assert!(
                    !bundle.join(&bundle_root).exists(),
                    "excluded root {} was copied into the bundle",
                    root.path
                );
            }
        }
    }
    assert!(manifest.coverage.included.iter().any(|entry| {
        entry.root == BackupCoverageRoot::WorkDir
            && entry.path == MANAGED_WORKSPACES_DIR
            && entry.kind == BackupCoverageKind::Directory
            && entry.included
            && entry.exclusion_reason.is_none()
    }));
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
async fn data_and_work_root_overlap_captures_conversations_once_through_work_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let bundle = dir.path().join("backup.nomifun");
    let destination = dir.path().join("restored");
    std::fs::create_dir_all(data_dir.join(MANAGED_WORKSPACES_DIR).join("managed")).unwrap();
    std::fs::write(
        data_dir
            .join(MANAGED_WORKSPACES_DIR)
            .join("managed/overlap.txt"),
        b"captured once",
    )
    .unwrap();
    let database = init_database(&data_dir.join("nomifun-backend.db"))
        .await
        .unwrap();

    let manifest = create_backup_bundle_with_sources(
        &database,
        &bundle,
        &generate_id(),
        BackupObjectGraph::full_database(),
        BackupSource::new(&data_dir, &data_dir),
    )
    .await
    .unwrap();
    database.close().await;

    let work_payload = format!(
        "{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}/managed/overlap.txt"
    );
    let data_prefix = format!("{BUNDLE_DATA_DIR}/{MANAGED_WORKSPACES_DIR}");
    assert_eq!(
        manifest
            .files
            .iter()
            .filter(|entry| entry.path == work_payload)
            .count(),
        1,
        "the shared physical conversations root must be captured exactly once"
    );
    assert!(
        manifest
            .files
            .iter()
            .all(|entry| entry.path != data_prefix
                && !entry.path.starts_with(&format!("{data_prefix}/"))),
        "conversations must never be duplicated through the data namespace"
    );
    assert!(
        manifest
            .directories
            .iter()
            .all(|path| path != &data_prefix && !path.starts_with(&format!("{data_prefix}/")))
    );
    assert!(bundle.join(&work_payload).is_file());
    assert!(!bundle.join(&data_prefix).exists());

    let registry_exclusion_reason = managed_dataset_roots()
        .find_map(|root| {
            if root.path == MANAGED_WORKSPACES_DIR {
                match root.backup {
                    BackupPolicy::Exclude(reason) => Some(reason),
                    BackupPolicy::Include => None,
                }
            } else {
                None
            }
        })
        .expect("data-dir conversations must be explicitly excluded");
    let data_coverage = manifest
        .coverage
        .excluded
        .iter()
        .find(|entry| {
            entry.root == BackupCoverageRoot::DataDir
                && entry.path == MANAGED_WORKSPACES_DIR
        })
        .expect("manifest must explain the data-dir conversations exclusion");
    assert_eq!(
        data_coverage.exclusion_reason.as_deref(),
        Some(registry_exclusion_reason)
    );
    assert!(manifest.coverage.included.iter().any(|entry| {
        entry.root == BackupCoverageRoot::WorkDir
            && entry.path == MANAGED_WORKSPACES_DIR
            && entry.kind == BackupCoverageKind::Directory
            && entry.included
            && entry.exclusion_reason.is_none()
    }));
    assert_eq!(verify_backup_bundle(&bundle).unwrap(), manifest);

    restore_backup_data_dir(&bundle, &destination)
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(
            destination
                .join(MANAGED_WORKSPACES_DIR)
                .join("managed/overlap.txt")
        )
        .unwrap(),
        "captured once"
    );
}

#[tokio::test]
async fn verification_rejects_legacy_format_and_coverage_drift() {
    let dir = tempfile::tempdir().unwrap();
    let database = init_database(&dir.path().join("source.db")).await.unwrap();
    let bundle = dir.path().join("backup.nomifun");
    create_backup_bundle(
        &database,
        &bundle,
        &generate_id(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();

    let manifest_path = bundle.join(MANIFEST_FILE);
    let original = std::fs::read(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["format_version"] = json!(1);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::InvalidManifest(_))
    ));

    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["coverage"]["excluded"]
        .as_array_mut()
        .unwrap()
        .pop();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::InvalidManifest(_))
    ));

    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["coverage"]["excluded"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::InvalidManifest(_))
    ));

    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["legacy_compatibility"] = json!(true);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let error = verify_backup_bundle(&bundle)
        .expect_err("strict v3 manifests must reject unknown compatibility fields");
    assert!(matches!(error, BackupError::InvalidManifest(_)));
}

#[tokio::test]
async fn pre_work_root_owner_v2_coverage_remains_verifiable_and_restorable() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let destination = dir.path().join("restored");
    let database = init_database(&source).await.unwrap();
    create_backup_bundle(
        &database,
        &bundle,
        &generate_id(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();

    let manifest_path = bundle.join(MANIFEST_FILE);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let excluded = manifest["coverage"]["excluded"].as_array_mut().unwrap();
    excluded.retain(|entry| {
        entry["path"] != WORK_ROOT_OWNER_FILE
            && entry["path"] != WORK_ROOT_BINDING_FILE
            && entry["path"] != "agent-process-registry.json"
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let verified = verify_backup_bundle(&bundle).unwrap();
    assert_eq!(verified.format_version, BACKUP_FORMAT_VERSION);
    assert!(
        verified
            .coverage
            .excluded
            .iter()
            .all(|entry| {
                entry.path != WORK_ROOT_OWNER_FILE
                    && entry.path != WORK_ROOT_BINDING_FILE
                    && entry.path != "agent-process-registry.json"
            })
    );

    let outcome = restore_backup_data_dir(&bundle, &destination)
    .await
    .unwrap();
    assert!(
        outcome
            .manifest
            .coverage
            .excluded
            .iter()
            .all(|entry| {
                entry.path != WORK_ROOT_OWNER_FILE
                    && entry.path != WORK_ROOT_BINDING_FILE
                    && entry.path != "agent-process-registry.json"
            })
    );
    assert!(destination.join("nomifun-backend.db").is_file());
    assert!(destination.join(DATASET_RECEIPT_FILE).is_file());

    manifest["coverage"]["excluded"][0]["exclusion_reason"] = json!("tampered");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        verify_backup_bundle(&bundle),
        Err(BackupError::InvalidManifest(_))
    ));
}

/// `BackupCoverage::validate` demands an exact match against the live
/// managed-dataset registry, and its only historical allowance is the
/// work-root host-control shape above. So dropping a root from that registry —
/// even one whose subsystem was deleted — silently invalidates every bundle
/// older releases produced: the coverage entry they recorded no longer has a
/// counterpart, and restore fails closed with `InvalidManifest`.
///
/// `browser-secrets` was published as an *included* data root by v0.3.1..=v0.3.7,
/// so it must keep that exact path, kind and policy even though the companion
/// credential subsystem is gone.
#[tokio::test]
async fn coverage_still_matches_bundles_published_by_earlier_releases() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let destination = dir.path().join("restored");
    let data_dir = dir.path().join("data");
    let work_dir = dir.path().join("work");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&work_dir).unwrap();
    let database = init_database(&source).await.unwrap();

    // Credential material a v0.3.7 install left behind.
    std::fs::create_dir_all(data_dir.join("browser-secrets")).unwrap();
    std::fs::write(
        data_dir.join("browser-secrets/vault.json"),
        b"legacy credential blob",
    )
    .unwrap();

    create_backup_bundle_with_sources(
        &database,
        &bundle,
        &generate_id(),
        BackupObjectGraph::full_database(),
        BackupSource {
            data_dir: &data_dir,
            work_dir: &work_dir,
        },
    )
    .await
    .unwrap();

    let verified = verify_backup_bundle(&bundle).unwrap();
    let entry = verified
        .coverage
        .included
        .iter()
        .find(|entry| {
            entry.root == BackupCoverageRoot::DataDir && entry.path == "browser-secrets"
        })
        .expect(
            "browser-secrets must stay an included data root: pre-v0.3.8 bundles \
             recorded it, and coverage validation is an exact match",
        );
    assert_eq!(entry.kind, BackupCoverageKind::Directory);

    restore_backup_data_dir(&bundle, &destination).await.unwrap();
    assert_eq!(
        std::fs::read(destination.join("browser-secrets/vault.json")).unwrap(),
        b"legacy credential blob",
        "a restore must not silently drop the legacy root it carried"
    );
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
         (conversation_id, user_id, name, type, extra, status, created_at, updated_at) \
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

    let outcome = restore_backup_data_dir(&bundle, restored_database.parent().unwrap())
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
    assert_eq!(
        prepare_v3_dataset(restored_database.parent().unwrap(), dir.path()).unwrap(),
        DatasetPreparation::Unchanged,
        "a restored v3 dataset must not be retired again on first startup"
    );
    assert!(
        restored_database.exists(),
        "dataset-generation validation must preserve the restored database"
    );

    let restored = open_read_only_pool(&restored_database).await;
    let restored_id: String =
        sqlx::query_scalar("SELECT conversation_id FROM conversations WHERE name = 'preserved'")
            .fetch_one(&restored)
            .await
            .unwrap();
    let restored_owner: String = sqlx::query_scalar(
        "SELECT owner_user_id FROM installation_identity WHERE singleton_key = 'installation'",
    )
    .fetch_one(&restored)
    .await
    .unwrap();
    assert_eq!(restored_id, conversation_id);
    assert_eq!(restored_owner, source_owner);
    restored.close().await;

    assert!(
        restore_backup_data_dir(&bundle, restored_database.parent().unwrap())
            .await
            .is_err(),
        "offline restore must never overwrite an existing dataset"
    );
}

#[tokio::test]
async fn restore_rebuilds_technical_ids_and_preserves_registered_business_id_references() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let bundle = dir.path().join("backup.nomifun");
    let destination = dir.path().join("restored");
    let database = init_database(&source).await.unwrap();
    let provider_id = generate_id();
    sqlx::query(
        "INSERT INTO providers \
         (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, created_at, updated_at) \
         VALUES (?, 'openai', 'restore provider', 'https://example.invalid', 'bearer', '', 1, 1)",
    )
    .bind(&provider_id)
    .execute(database.pool())
    .await
    .unwrap();

    // These values intentionally exercise SQLite technical-id remapping;
    // the product identity is always `requirement_id`.
    let discarded_requirement_technical_id: i64 = sqlx::query_scalar(
        "INSERT INTO requirements \
         (requirement_id, display_no, title, tag, created_at, updated_at) \
         VALUES (?, 1, 'discarded', 'restore', 1, 1) RETURNING id",
    )
    .bind(generate_id())
    .fetch_one(database.pool())
    .await
    .unwrap();
    sqlx::query("DELETE FROM requirements WHERE id = ?")
        .bind(discarded_requirement_technical_id)
        .execute(database.pool())
        .await
        .unwrap();
    let source_requirement_business_id = generate_id();
    let source_requirement_technical_id: i64 = sqlx::query_scalar(
        "INSERT INTO requirements \
         (requirement_id, display_no, title, tag, created_at, updated_at) \
         VALUES (?, 2, 'restored requirement', 'restore', 2, 2) RETURNING id",
    )
    .bind(&source_requirement_business_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let attachment_id = generate_id();
    sqlx::query(
        "INSERT INTO attachments \
         (attachment_id, requirement_id, file_name, rel_path, mime, size_bytes, created_at) \
         VALUES (?, ?, 'proof.txt', 'attachments/proof.txt', 'text/plain', 5, 2)",
    )
    .bind(&attachment_id)
    .bind(&source_requirement_business_id)
    .execute(database.pool())
    .await
    .unwrap();
    let source_attachment_parent: String = sqlx::query_scalar(
        "SELECT requirement_id FROM attachments WHERE attachment_id = ?",
    )
    .bind(&attachment_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let source_requirement_lookup: String = sqlx::query_scalar(
        "SELECT requirement_id FROM requirements WHERE requirement_id = ?",
    )
    .bind(&source_requirement_business_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(source_attachment_parent, source_requirement_lookup);

    let task_project_id = generate_id();
    let task_node_id = generate_id();
    let task_document = json!({
        "schema": "nomifun.creative-studio/v1",
        "projectId": task_project_id,
        "nodes": []
    });
    sqlx::query(
        "INSERT INTO creative_studio_projects \
         (project_id, title, revision, node_count, connection_count, document_json, created_at, updated_at) \
         VALUES (?, 'Backup task project', 1, 0, 0, ?, 1, 1)",
    )
    .bind(&task_project_id)
    .bind(task_document.to_string())
    .execute(database.pool())
    .await
    .unwrap();

    let discarded_creation_task_id = generate_id();
    let discarded_creation_task_technical_id: i64 = sqlx::query_scalar(
        "INSERT INTO creation_tasks \
         (creation_task_id, project_id, node_id, provider_id, model, capability, params, status, \
          submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'model', 'image', '{}', 'queued', 1, \
          '{\"backup_fixture\":\"discarded\"}') RETURNING id",
    )
    .bind(&discarded_creation_task_id)
    .bind(&task_project_id)
    .bind(&task_node_id)
    .bind(&provider_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    sqlx::query("DELETE FROM creation_tasks WHERE id = ?")
        .bind(discarded_creation_task_technical_id)
        .execute(database.pool())
        .await
        .unwrap();
    let source_creation_task_id = generate_id();
    let source_creation_task_technical_id: i64 = sqlx::query_scalar(
        "INSERT INTO creation_tasks \
         (creation_task_id, project_id, node_id, provider_id, model, capability, params, status, \
          submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'model', 'image', '{}', 'done', 2, \
          '{\"backup_fixture\":\"source\"}') RETURNING id",
    )
    .bind(&source_creation_task_id)
    .bind(&task_project_id)
    .bind(&task_node_id)
    .bind(&provider_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let asset_id = generate_id();
    sqlx::query(
        "INSERT INTO workshop_assets \
         (asset_id, kind, title, origin, created_at, updated_at) \
         VALUES (?, 'image', 'restored asset', ?, 2, 2)",
    )
    .bind(&asset_id)
    .bind(json!({"creation_task_id": source_creation_task_id.clone()}).to_string())
    .execute(database.pool())
    .await
    .unwrap();

    create_backup_bundle(
        &database,
        &bundle,
        &generate_id(),
        BackupObjectGraph::full_database(),
    )
    .await
    .unwrap();
    database.close().await;

    restore_backup_data_dir(&bundle, &destination)
    .await
    .unwrap();

    let restored = open_read_only_pool(&destination.join("nomifun-backend.db")).await;
    let restored_requirement_technical_id: i64 = sqlx::query_scalar(
        "SELECT id FROM requirements WHERE title = 'restored requirement'",
    )
    .fetch_one(&restored)
    .await
    .unwrap();
    let attachment_requirement_id: String =
        sqlx::query_scalar("SELECT requirement_id FROM attachments WHERE attachment_id = ?")
            .bind(&attachment_id)
            .fetch_one(&restored)
            .await
            .unwrap();
    let restored_creation_task_technical_id: i64 =
        sqlx::query_scalar("SELECT id FROM creation_tasks WHERE status = 'done'")
            .fetch_one(&restored)
            .await
            .unwrap();
    let asset_creation_task_id: String = sqlx::query_scalar(
        "SELECT json_extract(origin, '$.creation_task_id') \
         FROM workshop_assets WHERE asset_id = ?",
    )
    .bind(&asset_id)
    .fetch_one(&restored)
    .await
    .unwrap();
    assert_ne!(
        restored_requirement_technical_id,
        source_requirement_technical_id
    );
    assert_eq!(attachment_requirement_id, source_requirement_business_id);
    assert_ne!(
        restored_creation_task_technical_id,
        source_creation_task_technical_id
    );
    assert_eq!(asset_creation_task_id, source_creation_task_id);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT creation_task_id FROM creation_tasks WHERE status = 'done'",
        )
        .fetch_one(&restored)
        .await
        .unwrap(),
        source_creation_task_id
    );
    assert!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT json_extract(origin, '$.task_id') \
             FROM workshop_assets WHERE asset_id = ?",
        )
        .bind(&asset_id)
        .fetch_one(&restored)
        .await
        .unwrap()
        .is_none(),
        "technical creation_tasks.id must never be persisted in workshop_assets.origin"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT attachment_id FROM attachments WHERE file_name = 'proof.txt'",
        )
        .fetch_one(&restored)
        .await
        .unwrap(),
        attachment_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT asset_id FROM workshop_assets WHERE title = 'restored asset'",
        )
        .fetch_one(&restored)
        .await
        .unwrap(),
        asset_id
    );
    restored.close().await;
}

#[tokio::test]
async fn workshop_asset_origin_rejects_legacy_and_noncanonical_creation_task_references() {
    for (label, origin) in [
        ("legacy integer", json!({"task_id": 1})),
        ("legacy numeric string", json!({"task_id": "1"})),
        (
            "legacy field with UUIDv7",
            json!({"task_id": generate_id()}),
        ),
        ("integer business field", json!({"creation_task_id": 1})),
        (
            "numeric-string business field",
            json!({"creation_task_id": "1"}),
        ),
        (
            "prefixed UUIDv7 business field",
            json!({"creation_task_id": format!("task_{}", generate_id())}),
        ),
        (
            "UUIDv4 business field",
            json!({"creation_task_id": "550e8400-e29b-41d4-a716-446655440000"}),
        ),
        (
            "uppercase UUIDv7 business field",
            json!({"creation_task_id": generate_id().to_ascii_uppercase()}),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let database = init_database(&dir.path().join("source.db")).await.unwrap();
        let error = sqlx::query(
            "INSERT INTO workshop_assets \
             (asset_id, kind, title, origin, created_at, updated_at) \
             VALUES (?, 'image', ?, ?, 1, 1)",
        )
        .bind(generate_id())
        .bind(label)
        .bind(origin.to_string())
        .execute(database.pool())
        .await
        .expect_err("retired or noncanonical CreationTask origin references must fail");
        let message = error.to_string();
        assert!(
            message.contains("CHECK constraint failed")
                || message.contains("unsupported creative asset origin id key"),
            "{label}: unexpected database error: {message}"
        );
        database.close().await;
    }
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
    for root in managed_dataset_roots() {
        if root.backup != BackupPolicy::Include || root.path == ENCRYPTION_KEY_FILE {
            continue;
        }
        let path = source_root.join(root.path);
        match root.kind {
            DatasetRootKind::File => {
                std::fs::write(&path, format!("restored file {}", root.path)).unwrap();
            }
            DatasetRootKind::Directory => {
                std::fs::create_dir_all(path.join("empty")).unwrap();
                std::fs::write(
                    path.join("payload.txt"),
                    format!("restored directory {}", root.path),
                )
                .unwrap();
            }
        }
    }
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

    let outcome = restore_backup_data_dir(&bundle, &destination)
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
    for root in REQUIRED_PORTABLE_DIRECTORY_ROOTS {
        assert_eq!(
            std::fs::read_to_string(destination.join(root).join("payload.txt")).unwrap(),
            format!("restored directory {root}"),
            "required portable root {root} was not restored"
        );
        assert!(
            destination.join(root).join("empty").is_dir(),
            "empty directory in required portable root {root} was not restored"
        );
    }
    for root in managed_dataset_roots() {
        if root.backup != BackupPolicy::Include {
            continue;
        }
        let path = destination.join(root.path);
        match root.kind {
            DatasetRootKind::File => assert!(path.is_file(), "missing restored {}", root.path),
            DatasetRootKind::Directory => {
                assert!(
                    path.join("payload.txt").is_file(),
                    "missing restored payload for {}",
                    root.path
                );
                assert!(
                    path.join("empty").is_dir(),
                    "missing restored empty directory for {}",
                    root.path
                );
            }
        }
    }
    assert_eq!(
        std::fs::read_to_string(destination.join(STORAGE_GENERATION_FILE)).unwrap(),
        outcome.destination_storage_generation
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(destination.join(DATASET_RECEIPT_FILE)).unwrap())
            .unwrap();
    assert_eq!(receipt["contract_version"], 3);
    assert_eq!(
        receipt["generation"],
        outcome.destination_storage_generation
    );

    let corrupt_bundle = dir.path().join("corrupt.nomifun");
    copy_tree_for_test(&bundle, &corrupt_bundle);
    std::fs::write(corrupt_bundle.join(DATABASE_FILE), b"corrupt").unwrap();
    let untouched = dir.path().join("untouched");
    assert!(
        restore_backup_data_dir(&corrupt_bundle, &untouched)
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
    let error = restore_backup_data_dir(&bundle, &destination)
    .await
    .unwrap_err();
    assert!(
        format!("{error}").contains("v3 schema product-table registry mismatch"),
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
    sqlx::query("DELETE FROM installation_identity")
        .execute(&bundle_pool)
        .await
        .unwrap();
    bundle_pool.close().await;
    rewrite_database_manifest_entry(&bundle);
    verify_backup_bundle(&bundle).unwrap();

    let destination = dir.path().join("must-stay-absent");
    let error = restore_backup_data_dir(&bundle, &destination)
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
         (conversation_id, user_id, name, type, extra, status, created_at, updated_at) \
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
    let mut connection = bundle_pool.acquire().await.unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("UPDATE conversations SET conversation_id = 'conv_bad'")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    bundle_pool.close().await;
    rewrite_database_manifest_entry(&bundle);

    let destination = dir.path().join("must-stay-absent");
    let error = restore_backup_data_dir(&bundle, &destination)
    .await
    .unwrap_err();
    assert!(
        format!("{error}").contains("canonical")
            || format!("{error}").contains("CHECK constraint failed"),
        "unexpected validation failure: {error}"
    );
    assert!(!destination.exists());
}

#[tokio::test]
async fn restore_rejects_noncanonical_external_owner_ids_after_checksum_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("backup.nomifun");
    let database = init_database(&dir.path().join("source.db")).await.unwrap();
    sqlx::query(
        "INSERT INTO companion_access_token (companion_id, token_hash, created_at) \
         VALUES (?, 'hash', 1)",
    )
    .bind(nomifun_common::generate_id())
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
    let mut connection = bundle_pool.acquire().await.unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("UPDATE companion_access_token SET companion_id = 'companion_bad'")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    bundle_pool.close().await;
    rewrite_database_manifest_entry(&bundle);

    let destination = dir.path().join("must-stay-absent");
    let error = restore_backup_data_dir(&bundle, &destination)
    .await
    .unwrap_err();
    assert!(
        format!("{error}").contains("companion_access_token.companion_id")
            || format!("{error}").contains("canonical")
            || format!("{error}").contains("CHECK constraint failed"),
        "unexpected validation failure: {error}"
    );
    assert!(
        !destination.exists(),
        "ExternalOwner must skip only parent existence, never UUID format"
    );
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
        restore_backup_data_dir(&bundle, &destination)
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
