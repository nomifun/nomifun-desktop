use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, ArtifactId, LocalizedMetadata,
    MiniAppAdditiveMigrationAction, MiniAppBridgeCallId, MiniAppId,
    MiniAppMigration, MiniAppMigrationColumn, MiniAppMigrationId, MiniAppReleaseArtifactV1,
    MiniAppReleaseRef, MiniAppResourceContract, MiniAppServiceLifecycle, PackageId, PackageRef,
    ResolvedMiniAppServiceSpec, StrictJsonValue, VersionString, MINIAPP_BRIDGE_CONTRACT_VERSION,
};
use nomifun_api_types::{
    ExportPluginRuntimeBackupRequest, ImportPluginRuntimeBackupRequest, PluginRuntimeKindDto, PluginRuntimeLifecycleDto,
};
use nomifun_db::{
    BeginMiniAppM1ImportAsNewParams, CreateMiniAppM1Params, FinishMiniAppM1ImportReadyParams,
    IMiniAppM1Repository, MiniAppM1ImportSource, MiniAppM1Kind, MiniAppReleaseArtifactRow,
    MiniAppReleaseRow, SqliteMiniAppM1Repository, init_database_memory, installation_owner_id,
};
use nomifun_plugin_platform::runtime::{
    PluginRuntimeCallCancellation, PluginRuntimeDatabaseQueryResult, PluginRuntimeDatabaseStatement,
    PluginRuntimeM1ApplicationService, PluginRuntimePlatformError, PluginRuntimePlatformResult,
    PluginRuntimeReleaseFileBytes, PluginRuntimeReleasePublishRequest, PluginRuntimeReleaseStore,
    PluginRuntimePrivateDatabasePort, PluginRuntimeServiceHostState, PluginRuntimeServiceRuntimeBinding,
    PluginRuntimeServiceSpecInput, PluginRuntimeServiceStoragePort, PluginRuntimeServiceStorageRequest,
    PluginRuntimeSourceStore, PluginRuntimeStaticBundleBuilder, PluginRuntimeStaticBundleInput,
    PluginRuntimeStaticServiceInput, PluginRuntimeStoredRelease, PluginRuntimeBackupStorage,
    SqlitePluginRuntimeManagedStorage,
};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug)]
struct BackupRuntime {
    storage: Arc<SqlitePluginRuntimeManagedStorage>,
}

#[async_trait]
impl PluginRuntimeServiceRuntimeBinding for BackupRuntime {
    async fn resolve_spec(
        &self,
        _input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedMiniAppServiceSpec> {
        Err(PluginRuntimePlatformError::Runtime(
            "Backup test runtime does not resolve Service Hosts".into(),
        ))
    }

    async fn bind_active(
        &self,
        _spec: ResolvedMiniAppServiceSpec,
        _enabled: bool,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn start(&self, _spec: ResolvedMiniAppServiceSpec) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn invoke(
        &self,
        _spec: &ResolvedMiniAppServiceSpec,
        _call_id: MiniAppBridgeCallId,
        _method: String,
        _payload: StrictJsonValue,
        _cancellation: PluginRuntimeCallCancellation,
        _now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        Err(PluginRuntimePlatformError::ServiceUnavailable(
            "Backup test runtime does not invoke Service Hosts".into(),
        ))
    }

    async fn cancel(&self, _miniapp_id: &MiniAppId, _call_id: &MiniAppBridgeCallId) {}

    async fn stop(&self, _miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn retry(&self, _miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn state(&self, _miniapp_id: &MiniAppId) -> Option<PluginRuntimeServiceHostState> {
        Some(PluginRuntimeServiceHostState::Stopped)
    }

    async fn maintain(&self, _now_ms: i64) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn register_module(
        &self,
        _miniapp_id: MiniAppId,
        _release_digest: nomifun_agent_contracts::DigestHex,
        _module_path: PathBuf,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn export_backup_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeBackupStorage> {
        self.storage
            .export_backup_storage(
                owner_user_id,
                miniapp_id,
                uses_files,
                uses_private_database,
            )
            .await
    }

    async fn import_backup_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: PluginRuntimeBackupStorage,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<()> {
        self.storage
            .import_backup_storage(
                owner_user_id,
                miniapp_id,
                storage,
                uses_files,
                uses_private_database,
            )
            .await
    }
}

#[tokio::test]
async fn service_whole_app_backup_roundtrips_files_private_sqlite_and_migration_ledger() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> =
        Arc::new(SqliteMiniAppM1Repository::new(database.pool().clone()));
    let root = tempfile::tempdir().unwrap();

    let source_store = Arc::new(PluginRuntimeSourceStore::new(root.path().join("source")).unwrap());
    let release_store = Arc::new(PluginRuntimeReleaseStore::new(root.path().join("release")).unwrap());
    let storage = Arc::new(
        SqlitePluginRuntimeManagedStorage::new(root.path().join("managed"), database.pool().clone())
            .unwrap(),
    );
    let application = PluginRuntimeM1ApplicationService::new_with_stores(
        repository.clone(),
        source_store,
        release_store.clone(),
    )
    .unwrap();
    application
        .install_service_runtime(Arc::new(BackupRuntime {
            storage: storage.clone(),
        }))
        .await;

    let miniapp_id = Uuid::now_v7().to_string();
    let project_id = Uuid::now_v7().to_string();
    let operation_id = Uuid::now_v7().to_string();
    let artifact = service_artifact();
    let release_id = Uuid::now_v7().to_string();
    let source_snapshot_digest = digest_bytes(b"runtime-only-source");

    let begun = repository
        .begin_import_as_new(&BeginMiniAppM1ImportAsNewParams {
            create: CreateMiniAppM1Params {
                owner_user_id: owner.clone(),
                miniapp_id: miniapp_id.clone(),
                project_id: project_id.clone(),
                expected_library_revision: 0,
                display_name: "Service backup source".into(),
                description: Some("Service backup fixture".into()),
                icon_asset_id: None,
                kind: MiniAppM1Kind::Service,
                materialized_catalog_digest: digest_bytes(b"miniapp-m1-empty-catalog")
                    .as_ref()
                    .to_owned(),
                config_schema_json: r#"{"type":"object"}"#.into(),
                config_json: "{}".into(),
                created_at: 1,
            },
            operation_id: operation_id.clone(),
            source: MiniAppM1ImportSource::RuntimeOnly,
            bounded_log_tail: vec!["service backup fixture started".into()],
            started_at_ms: 1,
        })
        .await
        .unwrap();

    let published = release_store
        .publish(PluginRuntimeReleasePublishRequest::service(
            nomifun_plugin_platform::runtime::PluginRuntimeSourceScope::new(
                &owner,
                &miniapp_id,
                &project_id,
            )
            .unwrap(),
            source_snapshot_digest,
            artifact.manifest.payload.dependency_lock_digest.clone(),
            1,
            artifact.clone(),
            release_files(&artifact),
        ))
        .unwrap()
        .stored;

    let release_ref = MiniAppReleaseRef {
        release_id: release_id.clone().into(),
        artifact_id: artifact.artifact_id.clone(),
        release_digest: artifact.artifact_digest.clone(),
        manifest_digest: artifact.manifest.payload_digest.clone(),
    };
    let ready = nomifun_agent_contracts::MiniAppReadyRelease {
        miniapp_id: miniapp_id.clone().into(),
        release: release_ref.clone(),
        origin_operation_id: operation_id.clone().into(),
        origin: nomifun_agent_contracts::MiniAppReadyOrigin::Import,
        source_lineage: nomifun_agent_contracts::MiniAppSourceLineage::RuntimeOnly,
        matching_service_test_receipt: None,
        created_at_ms: 2,
    };
    ready.validate_for_artifact(&artifact).unwrap();

    let finished = repository
        .finish_import_ready(&FinishMiniAppM1ImportReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: project_id.clone(),
            operation_id: operation_id.clone(),
            expected_library_revision: begun.snapshot.library_revision,
            expected_product_revision: begun.snapshot.product.product_revision,
            expected_pointer_revision: begun.snapshot.product.pointer_revision,
            expected_project_revision: begun.snapshot.project.project_revision,
            artifact: artifact_row(&owner, &artifact, &published),
            release: release_row(
                &owner,
                &miniapp_id,
                &artifact,
                &release_id,
                &operation_id,
                &ready,
            ),
            bounded_log_tail: vec!["service backup fixture ready".into()],
            finished_at_ms: 3,
        })
        .await
        .unwrap();
    assert_eq!(
        finished.product.ready_release_id.as_deref(),
        Some(release_id.as_str())
    );
    assert_eq!(finished.project.source_state, "runtime_only");

    let miniapp = MiniAppId::from(miniapp_id.clone());
    let resolved = storage
        .resolve_service_storage(&owner, &miniapp, true, true)
        .await
        .unwrap();
    let private_database = resolved
        .descriptor
        .private_database
        .as_ref()
        .unwrap()
        .clone();
    let migration = migration();
    let ledger = PluginRuntimeServiceStoragePort::apply_additive_migrations(
            storage.as_ref(),
            &owner,
            &miniapp,
            &resolved.descriptor,
            &private_database.migration_ledger_digest,
            &release_ref,
            std::slice::from_ref(&migration),
            10,
        )
        .await
        .unwrap();
    assert_eq!(ledger.entries.len(), 1);
    assert_eq!(ledger.entries[0].release.release_id, release_ref.release_id);

    let resolved = storage
        .resolve_service_storage(&owner, &miniapp, true, true)
        .await
        .unwrap();
    storage
        .handle_service_request(
            &miniapp,
            &resolved.descriptor,
            PluginRuntimeServiceStorageRequest::DatabaseExecute {
                statement: PluginRuntimeDatabaseStatement {
                    sql: "INSERT INTO backup_state (id, value) VALUES (?, ?)".into(),
                    parameters: StrictJsonValue(json!(["source-row", 42])),
                },
            },
            PluginRuntimeCallCancellation::default(),
        )
        .await
        .unwrap();
    let files_root = PathBuf::from(
        &resolved
            .descriptor
            .files_dir
            .as_ref()
            .unwrap()
            .absolute_path,
    );
    fs::write(files_root.join("state.txt"), b"source-file").unwrap();

    let snapshot = repository
        .get(&owner, &miniapp_id)
        .await
        .unwrap()
        .unwrap();
    let destination = root.path().join("service-backup");
    let exported = application
        .export_backup(
            &owner,
            ExportPluginRuntimeBackupRequest {
                miniapp_id: miniapp_id.clone(),
                expected_product_revision: snapshot.product.product_revision.try_into().unwrap(),
                expected_lifecycle: PluginRuntimeLifecycleDto::Disabled,
                expected_pointer_revision: snapshot.product.pointer_revision.try_into().unwrap(),
                expected_config_revision: snapshot.product.config_revision.try_into().unwrap(),
                expected_credential_bindings_revision: snapshot
                    .product
                    .credential_bindings_revision
                    .try_into()
                    .unwrap(),
                destination_path: destination.display().to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        exported.state,
        nomifun_api_types::DurableOperationStateDto::Succeeded
    );
    assert!(destination.join("storage/private.sqlite").is_file());
    assert!(destination.join("storage/migration-ledger.json").is_file());
    assert!(destination.join("storage/files/state.txt").is_file());

    let imported = application
        .import_backup(
            &owner,
            ImportPluginRuntimeBackupRequest {
                expected_library_revision: application.library(&owner).await.unwrap().library_revision,
                source_path: destination.display().to_string(),
                expected_backup_metadata_digest: metadata_digest(&destination),
                display_name: "Service backup copy".into(),
            },
        )
        .await
        .unwrap();
    let imported_id = imported.miniapp.miniapp_id.clone();
    assert_ne!(imported_id, miniapp_id);
    assert_eq!(imported.miniapp.kind, PluginRuntimeKindDto::Service);
    assert_eq!(imported.miniapp.lifecycle, PluginRuntimeLifecycleDto::Disabled);
    assert!(imported.miniapp.releases.ready.is_some());

    let imported_miniapp = MiniAppId::from(imported_id.clone());
    let imported_storage = storage
        .resolve_service_storage(&owner, &imported_miniapp, true, true)
        .await
        .unwrap();
    let imported_database = imported_storage
        .descriptor
        .private_database
        .as_ref()
        .unwrap();
    let imported_ledger = storage
        .ledger(&imported_miniapp, &imported_database.handle_id)
        .await
        .unwrap();
    assert_eq!(imported_ledger.miniapp_id, imported_miniapp);
    assert_eq!(imported_ledger.entries.len(), 1);
    assert_ne!(
        imported_ledger.entries[0].release.release_id,
        release_ref.release_id
    );
    assert_eq!(
        imported_ledger.entries[0].release.release_id.as_ref(),
        imported.miniapp.releases.ready.as_ref().unwrap().release_id.as_str()
    );
    assert_eq!(
        imported_ledger.entries[0].release.release_digest,
        release_ref.release_digest
    );
    assert_eq!(
        imported_ledger.entries[0].release.artifact_id,
        release_ref.artifact_id
    );

    let query = storage
        .handle_service_request(
            &imported_miniapp,
            &imported_storage.descriptor,
            PluginRuntimeServiceStorageRequest::DatabaseQuery {
                statement: PluginRuntimeDatabaseStatement {
                    sql: "SELECT id, value FROM backup_state WHERE id = ?".into(),
                    parameters: StrictJsonValue(json!(["source-row"])),
                },
            },
            PluginRuntimeCallCancellation::default(),
        )
        .await
        .unwrap();
    let query: PluginRuntimeDatabaseQueryResult = serde_json::from_value(query.0).unwrap();
    assert_eq!(
        query.rows,
        vec![StrictJsonValue(json!({"id": "source-row", "value": 42}))]
    );

    let imported_file = PathBuf::from(
        &imported_storage
            .descriptor
            .files_dir
            .as_ref()
            .unwrap()
            .absolute_path,
    )
    .join("state.txt");
    assert_eq!(fs::read(imported_file).unwrap(), b"source-file");
}

fn service_artifact() -> MiniAppReleaseArtifactV1 {
    let migration = migration();
    PluginRuntimeStaticBundleBuilder::new()
        .build(PluginRuntimeStaticBundleInput {
            artifact_id: ArtifactId::from(Uuid::now_v7().to_string()),
            display: LocalizedMetadata {
                name: "Service backup".into(),
                description: "Service backup fixture".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            ui_index_html: br#"<!doctype html><html><body>backup</body></html>"#.to_vec(),
            ui_assets: Vec::new(),
            service: Some(PluginRuntimeStaticServiceInput {
                main_mjs: br#"export async function start() {
  return { async invoke() { return null; }, async dispose() {} };
}
"#
                .to_vec(),
                lifecycle: MiniAppServiceLifecycle::OnDemand,
                uses_files: true,
                uses_private_database: true,
                service_contract_digest: digest_bytes(b"service-contract"),
                runtime_requirements_digest: digest_bytes(b"runtime-requirements"),
            }),
            package_json: None,
            dependency_lock_digest: digest_bytes(b"service-backup-lock"),
            dependency_graph_digest: digest_bytes(b"service-backup-graph"),
            config_schema: StrictJsonValue(json!({"type": "object"})),
            credential_slots: Vec::new(),
            resource_contract: MiniAppResourceContract::default(),
            schemas: BTreeMap::new(),
            bridge_contract_digest: digest_bytes(MINIAPP_BRIDGE_CONTRACT_VERSION.as_bytes()),
            contribution_package: PackageRef {
                id: PackageId::from("miniapp.service-backup"),
                version: VersionString::from("1.0.0"),
            },
            contributions: Default::default(),
            migrations: vec![migration],
        })
        .unwrap()
}

fn migration() -> MiniAppMigration {
    MiniAppMigration::new(
        MiniAppMigrationId::from("001_create_backup_state"),
        vec![MiniAppAdditiveMigrationAction::CreateTable {
            table_name: "backup_state".into(),
            columns: vec![
                MiniAppMigrationColumn {
                    name: "id".into(),
                    declared_type: "TEXT".into(),
                    nullable: false,
                    default_literal: None,
                },
                MiniAppMigrationColumn {
                    name: "value".into(),
                    declared_type: "INTEGER".into(),
                    nullable: false,
                    default_literal: Some("0".into()),
                },
            ],
            primary_key_columns: vec!["id".into()],
        }],
    )
    .unwrap()
}

fn release_files(artifact: &MiniAppReleaseArtifactV1) -> Vec<PluginRuntimeReleaseFileBytes> {
    artifact
        .files
        .iter()
        .map(|file| {
            let bytes = if file.normalized_relative_path == "ui/index.html" {
                let source = br#"<!doctype html><html><body>backup</body></html>"#;
                nomifun_plugin_platform::runtime::materialize_surface_entrypoint(source).unwrap()
            } else {
                br#"export async function start() {
  return { async invoke() { return null; }, async dispose() {} };
}
"#
                .to_vec()
            };
            PluginRuntimeReleaseFileBytes::new(file.normalized_relative_path.clone(), bytes)
        })
        .collect()
}

fn artifact_row(
    owner: &str,
    artifact: &MiniAppReleaseArtifactV1,
    published: &PluginRuntimeStoredRelease,
) -> MiniAppReleaseArtifactRow {
    MiniAppReleaseArtifactRow {
        id: 0,
        artifact_id: artifact.artifact_id.as_ref().to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
        manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
        artifact_record_json: canonical_string(artifact),
        managed_path: published.managed_relative_path.clone(),
        created_at: 2,
    }
}

fn release_row(
    owner: &str,
    miniapp_id: &str,
    artifact: &MiniAppReleaseArtifactV1,
    release_id: &str,
    operation_id: &str,
    ready: &nomifun_agent_contracts::MiniAppReadyRelease,
) -> MiniAppReleaseRow {
    MiniAppReleaseRow {
        id: 0,
        release_id: release_id.to_owned(),
        miniapp_id: miniapp_id.to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_id: artifact.artifact_id.as_ref().to_owned(),
        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
        manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
        release_digest: artifact.artifact_digest.as_ref().to_owned(),
        origin_kind: "import".into(),
        origin_operation_id: operation_id.to_owned(),
        source_kind: "runtime_only".into(),
        project_id: None,
        source_snapshot_digest: None,
        dependency_lock_digest: None,
        build_profile_version: None,
        build_generation: None,
        release_record_json: canonical_string(ready),
        created_at: 2,
    }
}

fn canonical_string<T: serde::Serialize>(value: &T) -> String {
    String::from_utf8(canonical_json_bytes(value).unwrap()).unwrap()
}

fn metadata_digest(root: &std::path::Path) -> String {
    let metadata: nomifun_agent_contracts::MiniAppWholeAppBackupMetadataV1 =
        serde_json::from_slice(&fs::read(root.join("metadata.json")).unwrap()).unwrap();
    metadata.metadata_digest().unwrap().as_ref().to_owned()
}
