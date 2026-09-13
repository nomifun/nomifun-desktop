use std::collections::BTreeMap;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ArtifactId, DigestHex, JavaScriptBuildProfile, LocalizedMetadata,
    MiniAppReleaseArtifactV1, MiniAppReleaseFile, MiniAppReleaseRef,
    MiniAppReleaseV1Manifest, MiniAppReadyOrigin, MiniAppReadyRelease,
    MiniAppAdditiveMigrationAction, MiniAppMigration, MiniAppMigrationColumn,
    MiniAppMigrationId, MiniAppServiceRuntimeFingerprint,
    MiniAppServiceTestCredentialMode, MiniAppServiceTestOutcome,
    MiniAppServiceTestReceipt, RuntimeInstallationId, RuntimeTarget,
    MiniAppResourceContract, MiniAppServiceLifecycle, MiniAppServiceReleaseDescriptor,
    MiniAppSourceLineage, MiniAppUiReleaseDescriptor, PackageId, PackageRef, StrictJsonValue,
    VersionString,
    MINIAPP_BRIDGE_CONTRACT_VERSION, MINIAPP_M1_SCHEMA_VERSION,
    MINIAPP_RELEASE_PROFILE_VERSION, MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
    MINIAPP_SERVICE_SDK_CONTRACT_VERSION, MINIAPP_SERVICE_TEST_CONTRACT_VERSION,
    canonical_ui_tree_digest, digest_bytes, digest_payload,
};
use nomifun_db::{
    AbortMiniAppSourceMutationParams, BeginMiniAppM1DeleteParams,
    BeginMiniAppM1ImportAsNewParams, BeginMiniAppSourceMutationParams,
    CancelMiniAppM1BuildOperationParams, CloseMiniAppM1SurfaceSessionParams,
    CommitMiniAppM1LifecycleParams,
    CreateMiniAppM1Params, CreateMiniAppM1WithSourceParams,
    ExecuteMiniAppM1SurfaceKvParams,
    FailMiniAppM1DeleteParams, FinalizeMiniAppM1DeleteParams,
    FinalizeMiniAppSourceMutationParams,
    FinishMiniAppM1BuildAndRecordReadyParams, FinishMiniAppM1BuildOperationParams,
    FinishMiniAppM1ExportOperationParams, FinishMiniAppM1ImportReadyParams,
    IMiniAppM1Repository, MiniAppM1AutoPublishGuard, MiniAppM1ImportSource, MiniAppM1Kind,
    MiniAppM1ManagedSourceLineage, MiniAppM1ProjectSourceState,
    MiniAppM1Snapshot, MiniAppM1SurfaceKvOperation, MiniAppM1SurfaceKvResult,
    MiniAppReleaseArtifactRow, MiniAppReleaseRow,
    OpenMiniAppM1SurfaceSessionParams, ProductOperationState,
    PublishMiniAppM1ReadyParams, ResolveMiniAppM1SurfaceSessionParams,
    RecordMiniAppM1ServiceTestReceiptParams,
    RestartMiniAppM1DeleteParams, RestoreMiniAppM1Params,
    RollbackMiniAppM1PreviousParams, SetMiniAppM1AutoPublishParams,
    SqliteMiniAppM1Repository,
    StartMiniAppM1BackupExportParams, StartMiniAppM1BuildOperationParams,
    StartMiniAppM1ExportOperationParams,
    TrashMiniAppM1Params, UpdateMiniAppM1ProjectSourceParams, installation_owner_id,
};
use serde_json::json;
use sqlx::migrate::{Migrate, Migrator};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const MINIAPP_ID: &str = "0190f5fe-7c00-7000-8000-000000000101";
const PROJECT_ID: &str = "0190f5fe-7c00-7000-8000-000000000102";
const SERVICE_MINIAPP_ID: &str = "0190f5fe-7c00-7000-8000-000000000103";
const SERVICE_PROJECT_ID: &str = "0190f5fe-7c00-7000-8000-000000000104";

struct MiniAppTestDatabase {
    pool: nomifun_db::SqlitePool,
}

impl MiniAppTestDatabase {
    fn pool(&self) -> &nomifun_db::SqlitePool {
        &self.pool
    }
}

async fn init_miniapp_test_database() -> MiniAppTestDatabase {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR.iter() {
        connection.apply(migration).await.unwrap();
    }
    drop(connection);
    let owner = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, 'admin', '', '', 1, 1)",
    )
    .bind(&owner)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO installation_identity (singleton_key, owner_user_id)
         VALUES ('installation', ?)",
    )
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();
    MiniAppTestDatabase { pool }
}

async fn insert_other_owner(pool: &nomifun_db::SqlitePool) -> String {
    let owner = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(&owner)
    .bind(&owner)
    .execute(pool)
    .await
    .unwrap();
    owner
}

fn managed_source(
    path: &str,
    source_digest_char: char,
    lock_digest_char: char,
    build_generation: i64,
) -> MiniAppM1ManagedSourceLineage {
    MiniAppM1ManagedSourceLineage {
        managed_source_path: path.to_owned(),
        source_head_digest: source_digest_char.to_string().repeat(64),
        dependency_lock_digest: lock_digest_char.to_string().repeat(64),
        build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.to_owned(),
        build_generation,
    }
}

fn create_params(
    owner: &str,
    miniapp_id: &str,
    project_id: &str,
    expected_library_revision: i64,
    kind: MiniAppM1Kind,
    created_at: i64,
) -> CreateMiniAppM1Params {
    CreateMiniAppM1Params {
        owner_user_id: owner.to_owned(),
        miniapp_id: miniapp_id.to_owned(),
        project_id: project_id.to_owned(),
        expected_library_revision,
        display_name: format!("MiniApp {miniapp_id}"),
        description: None,
        icon_asset_id: None,
        kind,
        materialized_catalog_digest: "a".repeat(64),
        config_schema_json: r#"{"type":"object"}"#.to_owned(),
        config_json: "{}".to_owned(),
        created_at,
    }
}

async fn start_build(
    repository: &SqliteMiniAppM1Repository,
    owner: &str,
    miniapp_id: &str,
    project_id: &str,
    project_revision: i64,
    source: &MiniAppM1ManagedSourceLineage,
    operation_id: &str,
    started_at_ms: i64,
) -> nomifun_db::ProductOperationRow {
    repository
        .start_build_operation(&StartMiniAppM1BuildOperationParams {
            owner_user_id: owner.to_owned(),
            miniapp_id: miniapp_id.to_owned(),
            project_id: project_id.to_owned(),
            operation_id: operation_id.to_owned(),
            expected_project_revision: project_revision,
            expected_source: source.clone(),
            bounded_log_tail: vec!["build started".to_owned()],
            started_at_ms,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn owner_scoped_library_create_and_project_source_cas_are_exact() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());

    let empty = repository.library(&owner).await.unwrap();
    assert_eq!(empty.library.revision, 0);
    assert!(empty.products.is_empty());

    let created = repository
        .create(&CreateMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_library_revision: 0,
            display_name: "First M1 App".to_owned(),
            description: Some("schema-only foundation".to_owned()),
            icon_asset_id: None,
            kind: MiniAppM1Kind::UiOnly,
            materialized_catalog_digest: "a".repeat(64),
            config_schema_json: r#"{"type":"object"}"#.to_owned(),
            config_json: "{}".to_owned(),
            created_at: 10,
        })
        .await
        .unwrap();
    assert_eq!(created.product.miniapp_id, MINIAPP_ID);
    assert_eq!(created.project.project_id, PROJECT_ID);
    assert_eq!(created.library_revision, 1);

    let edited = repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 1,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some("sources/owner/projects/project/source".into()),
            source_head_digest: Some("b".repeat(64)),
            dependency_lock_digest: Some("c".repeat(64)),
            build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.into()),
            build_generation: 1,
            updated_at: 20,
        })
        .await
        .unwrap();
    assert_eq!(edited.project_revision, 2);
    assert_eq!(edited.source_state, "editable");

    let stale = repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 1,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some("sources/owner/projects/project/source".into()),
            source_head_digest: Some("d".repeat(64)),
            dependency_lock_digest: Some("e".repeat(64)),
            build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.into()),
            build_generation: 2,
            updated_at: 21,
        })
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("CAS"));
}

#[tokio::test]
async fn source_mutation_intent_fences_the_exact_project_and_finalizes_once() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    repository
        .create(&create_params(
            &owner,
            MINIAPP_ID,
            PROJECT_ID,
            0,
            MiniAppM1Kind::UiOnly,
            10,
        ))
        .await
        .unwrap();
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 1,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some("sources/owner/miniapp/project/source".into()),
            source_head_digest: Some("b".repeat(64)),
            dependency_lock_digest: Some("c".repeat(64)),
            build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.into()),
            build_generation: 1,
            updated_at: 20,
        })
        .await
        .unwrap();

    let intent_id = Uuid::now_v7().to_string();
    let intent = repository
        .begin_source_mutation(&BeginMiniAppSourceMutationParams {
            intent_id: intent_id.clone(),
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_product_revision: 1,
            expected_project_revision: 2,
            expected_build_generation: 1,
            expected_source_digest: "b".repeat(64),
            next_source_digest: "d".repeat(64),
            next_build_generation: 2,
            created_at: 21,
        })
        .await
        .unwrap();
    assert_eq!(intent.intent_id, intent_id);

    let fenced_project = repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 2,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some("sources/owner/miniapp/project/source".into()),
            source_head_digest: Some("e".repeat(64)),
            dependency_lock_digest: Some("c".repeat(64)),
            build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.into()),
            build_generation: 2,
            updated_at: 22,
        })
        .await
        .unwrap_err();
    assert!(fenced_project.to_string().contains("fenced"));
    let fenced_product = sqlx::query(
        "UPDATE miniapp_products SET description = 'racing edit' WHERE miniapp_id = ?",
    )
    .bind(MINIAPP_ID)
    .execute(database.pool())
    .await
    .unwrap_err();
    assert!(fenced_product.to_string().contains("fenced"));

    let committed = repository
        .finalize_source_mutation(&FinalizeMiniAppSourceMutationParams {
            intent_id: intent_id.clone(),
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            updated_at: 22,
        })
        .await
        .unwrap();
    assert_eq!(committed.project.project_revision, 3);
    assert_eq!(committed.project.build_generation, 2);
    assert_eq!(committed.project.source_head_digest.as_deref(), Some("d".repeat(64).as_str()));
    assert_eq!(committed.library_revision, 3);
    assert!(repository
        .get_source_mutation_intent(&owner, MINIAPP_ID, PROJECT_ID)
        .await
        .unwrap()
        .is_none());
    let markers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_source_mutation_commits")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(markers, 0);
    assert!(repository
        .finalize_source_mutation(&FinalizeMiniAppSourceMutationParams {
            intent_id,
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            updated_at: 23,
        })
        .await
        .is_err());

    let abort_id = Uuid::now_v7().to_string();
    repository
        .begin_source_mutation(&BeginMiniAppSourceMutationParams {
            intent_id: abort_id.clone(),
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_product_revision: 1,
            expected_project_revision: 3,
            expected_build_generation: 2,
            expected_source_digest: "d".repeat(64),
            next_source_digest: "e".repeat(64),
            next_build_generation: 3,
            created_at: 23,
        })
        .await
        .unwrap();
    repository
        .abort_source_mutation(&AbortMiniAppSourceMutationParams {
            intent_id: abort_id,
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
        })
        .await
        .unwrap();
    assert!(repository.list_source_mutation_intents().await.unwrap().is_empty());
}

#[tokio::test]
async fn new_repository_never_reads_the_retired_miniapps_store() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    assert!(repository
        .get(&owner, MINIAPP_ID)
        .await
        .unwrap()
        .is_none());
    let old_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM miniapps")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(old_count, 0);
}

#[tokio::test]
async fn managed_import_as_new_commits_ready_and_export_operation_exactly() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let miniapp_id = Uuid::now_v7().to_string();
    let project_id = Uuid::now_v7().to_string();
    let operation_id = Uuid::now_v7().to_string();
    let source = managed_source(
        &format!(
            "sources/{owner}/miniapps/{miniapp_id}/projects/{project_id}/source"
        ),
        'b',
        'c',
        1,
    );
    let begun = repository
        .begin_import_as_new(&BeginMiniAppM1ImportAsNewParams {
            create: create_params(
                &owner,
                &miniapp_id,
                &project_id,
                0,
                MiniAppM1Kind::UiOnly,
                10,
            ),
            operation_id: operation_id.clone(),
            source: MiniAppM1ImportSource::Managed(source.clone()),
            bounded_log_tail: vec!["import started".into()],
            started_at_ms: 10,
        })
        .await
        .unwrap();
    assert_eq!(begun.snapshot.product.lifecycle, "disabled");
    assert_eq!(begun.snapshot.project.source_state, "editable");
    assert_eq!(begun.operation.state, "running");
    assert!(begun.snapshot.catalog_publication.is_none());

    let artifact_id = Uuid::now_v7().to_string();
    let release_id = Uuid::now_v7().to_string();
    let artifact = artifact(&owner, &artifact_id, "imported", "manifest", 11);
    let mut imported_release = release(
        &owner,
        &miniapp_id,
        &project_id,
        &artifact,
        &release_id,
        "imported",
        &operation_id,
        &source.source_head_digest,
        &source.dependency_lock_digest,
        11,
    );
    imported_release.origin_kind = "import".into();
    let mut ready: MiniAppReadyRelease =
        serde_json::from_str(&imported_release.release_record_json).unwrap();
    ready.origin = MiniAppReadyOrigin::Import;
    imported_release.release_record_json = String::from_utf8(
        nomifun_agent_contracts::canonical_json_bytes(&ready).unwrap(),
    )
    .unwrap();
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
            artifact,
            release: imported_release,
            bounded_log_tail: vec!["import ready committed".into()],
            finished_at_ms: 12,
        })
        .await
        .unwrap();
    assert_eq!(finished.product.ready_release_id.as_deref(), Some(release_id.as_str()));
    assert!(finished.product.active_release_id.is_none());
    assert!(finished.catalog_publication.is_none());
    let import_operation = repository
        .get_miniapp_operation(&owner, &miniapp_id, &operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(import_operation.state, "succeeded");

    let export_id = Uuid::now_v7().to_string();
    repository
        .start_export_operation(&StartMiniAppM1ExportOperationParams {
            owner_user_id: owner.clone(),
            miniapp_id: miniapp_id.clone(),
            operation_id: export_id.clone(),
            expected_product_revision: finished.product.product_revision,
            expected_pointer_revision: finished.product.pointer_revision,
            bounded_log_tail: vec!["export started".into()],
            started_at_ms: 13,
        })
        .await
        .unwrap();
    let exported = repository
        .finish_export_operation(&FinishMiniAppM1ExportOperationParams {
            owner_user_id: owner,
            miniapp_id,
            operation_id: export_id,
            bounded_log_tail: vec!["export completed".into()],
            finished_at_ms: 14,
        })
        .await
        .unwrap();
    assert_eq!(exported.state, "succeeded");
    assert_eq!(exported.progress_percent, Some(100));
}

#[tokio::test]
async fn ordinary_export_start_is_blocked_by_a_running_backup_export() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let miniapp_id = Uuid::now_v7().to_string();
    let project_id = Uuid::now_v7().to_string();
    let created = repository
        .create(&create_params(
            &owner,
            &miniapp_id,
            &project_id,
            0,
            MiniAppM1Kind::UiOnly,
            1,
        ))
        .await
        .unwrap();

    repository
        .start_backup_export(&StartMiniAppM1BackupExportParams {
            owner_user_id: owner.clone(),
            miniapp_id: miniapp_id.clone(),
            operation_id: Uuid::now_v7().to_string(),
            expected_product_revision: created.product.product_revision,
            expected_pointer_revision: created.product.pointer_revision,
            expected_config_revision: created.product.config_revision,
            expected_credential_bindings_revision: created
                .product
                .credential_bindings_revision,
            started_at_ms: 2,
        })
        .await
        .unwrap();

    let error = repository
        .start_export_operation(&StartMiniAppM1ExportOperationParams {
            owner_user_id: owner,
            miniapp_id,
            operation_id: Uuid::now_v7().to_string(),
            expected_product_revision: created.product.product_revision,
            expected_pointer_revision: created.product.pointer_revision,
            bounded_log_tail: vec!["share export started".into()],
            started_at_ms: 3,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("running operation"));
}

fn artifact(
    owner: &str,
    artifact_id: &str,
    artifact_seed: &str,
    _manifest_seed: &str,
    created_at: i64,
) -> MiniAppReleaseArtifactRow {
    let artifact_payload = fixture_artifact_payload(artifact_id, artifact_seed);
    let artifact_record_json = String::from_utf8(
        nomifun_agent_contracts::canonical_json_bytes(&artifact_payload).unwrap(),
    )
    .unwrap();
    MiniAppReleaseArtifactRow {
        id: 0,
        artifact_id: artifact_id.to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_digest: artifact_payload.artifact_digest.as_ref().to_owned(),
        manifest_digest: artifact_payload.manifest.payload_digest.as_ref().to_owned(),
        artifact_record_json,
        managed_path: format!(
            "artifacts/{}",
            artifact_payload.artifact_digest.as_ref()
        ),
        created_at,
    }
}

fn fixture_artifact_payload(
    artifact_id: &str,
    artifact_seed: &str,
) -> MiniAppReleaseArtifactV1 {
    fixture_artifact_payload_with_service(
        artifact_id,
        artifact_seed,
        false,
        false,
        false,
        false,
    )
}

fn fixture_service_artifact_payload(
    artifact_id: &str,
    artifact_seed: &str,
    uses_files: bool,
) -> MiniAppReleaseArtifactV1 {
    fixture_artifact_payload_with_service(
        artifact_id,
        artifact_seed,
        true,
        uses_files,
        false,
        false,
    )
}

fn fixture_artifact_payload_with_service(
    artifact_id: &str,
    artifact_seed: &str,
    has_service: bool,
    uses_files: bool,
    uses_private_database: bool,
    with_migration: bool,
) -> MiniAppReleaseArtifactV1 {
    let html = format!(
        "<!doctype html><html><body><main><h1>fixture-{artifact_seed}</h1></main></body></html>"
    )
    .into_bytes();
    let ui_file = MiniAppReleaseFile {
        normalized_relative_path: "ui/index.html".to_owned(),
        digest: digest_bytes(&html),
        size_bytes: html.len() as u64,
    };
    let files = if has_service {
        let service = format!("export default {{ invoke() {{ return '{artifact_seed}'; }} }};")
            .into_bytes();
        vec![
            ui_file,
            MiniAppReleaseFile {
                normalized_relative_path: "service/main.mjs".to_owned(),
                digest: digest_bytes(&service),
                size_bytes: service.len() as u64,
            },
        ]
    } else {
        vec![ui_file]
    };
    let config_schema = StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false
    }));
    let resource_contract = MiniAppResourceContract::default();
    let manifest = MiniAppReleaseV1Manifest {
        schema_version: VersionString::from(MINIAPP_M1_SCHEMA_VERSION),
        build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
        build_profile_version: VersionString::from(MINIAPP_RELEASE_PROFILE_VERSION),
        display: LocalizedMetadata {
            name: format!("Fixture {artifact_seed}"),
            description: "MiniApp repository fixture".to_owned(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        ui: Some(MiniAppUiReleaseDescriptor {
            entrypoint: "ui/index.html".to_owned(),
            entrypoint_digest: files[0].digest.clone(),
            ui_tree_digest: canonical_ui_tree_digest(&files).unwrap(),
        }),
        service: has_service.then(|| MiniAppServiceReleaseDescriptor {
            entrypoint: "service/main.mjs".to_owned(),
            module_digest: files[1].digest.clone(),
            lifecycle: MiniAppServiceLifecycle::OnDemand,
            uses_files,
            uses_private_database,
            service_contract_digest: digest_bytes(b"service-contract"),
            host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime_requirements_digest: digest_bytes(b"runtime-requirements"),
        }),
        dependency_lock_digest: DigestHex::from("c".repeat(64)),
        dependency_graph_digest: digest_bytes(
            format!("graph-{artifact_seed}").as_bytes(),
        ),
        config_schema: config_schema.clone(),
        config_schema_digest: digest_payload(&config_schema.0).unwrap(),
        credential_slots: Vec::new(),
        credential_slots_digest: digest_payload(&Vec::<
            nomifun_agent_contracts::CredentialSlotDeclaration,
        >::new())
        .unwrap(),
        resource_contract: resource_contract.clone(),
        resource_contract_digest: digest_payload(&resource_contract).unwrap(),
        schemas: BTreeMap::new(),
        bridge_contract_digest: digest_payload(&VersionString::from(
            MINIAPP_BRIDGE_CONTRACT_VERSION,
        ))
        .unwrap(),
        contribution_package: PackageRef {
            id: PackageId::from("miniapp.test"),
            version: VersionString::from("1.0.0"),
        },
        contributions: Default::default(),
        migrations: if with_migration {
            vec![MiniAppMigration::new(
                MiniAppMigrationId::from("001_create_state"),
                vec![MiniAppAdditiveMigrationAction::CreateTable {
                    table_name: "state".to_owned(),
                    columns: vec![MiniAppMigrationColumn {
                        name: "id".to_owned(),
                        declared_type: "INTEGER".to_owned(),
                        nullable: false,
                        default_literal: None,
                    }],
                    primary_key_columns: vec!["id".to_owned()],
                }],
            )
            .unwrap()]
        } else {
            Vec::new()
        },
    };
    MiniAppReleaseArtifactV1::new(
        ArtifactId::from(artifact_id),
        manifest,
        files,
    )
    .unwrap()
}

fn service_artifact(
    owner: &str,
    artifact_id: &str,
    artifact_seed: &str,
    created_at: i64,
    uses_files: bool,
    uses_private_database: bool,
    with_migration: bool,
) -> MiniAppReleaseArtifactRow {
    let artifact_payload = if uses_files || uses_private_database || with_migration {
        fixture_artifact_payload_with_service(
            artifact_id,
            artifact_seed,
            true,
            uses_files,
            uses_private_database,
            with_migration,
        )
    } else {
        fixture_service_artifact_payload(artifact_id, artifact_seed, false)
    };
    let artifact_record_json = String::from_utf8(
        nomifun_agent_contracts::canonical_json_bytes(&artifact_payload).unwrap(),
    )
    .unwrap();
    MiniAppReleaseArtifactRow {
        id: 0,
        artifact_id: artifact_id.to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_digest: artifact_payload.artifact_digest.as_ref().to_owned(),
        manifest_digest: artifact_payload.manifest.payload_digest.as_ref().to_owned(),
        artifact_record_json,
        managed_path: format!("artifacts/{}", artifact_payload.artifact_digest.as_ref()),
        created_at,
    }
}

fn release(
    owner: &str,
    miniapp_id: &str,
    project_id: &str,
    artifact: &MiniAppReleaseArtifactRow,
    release_id: &str,
    _release_seed: &str,
    operation_id: &str,
    source_digest: &str,
    lock_digest: &str,
    created_at: i64,
) -> MiniAppReleaseRow {
    let artifact_payload: MiniAppReleaseArtifactV1 =
        serde_json::from_str(&artifact.artifact_record_json).unwrap();
    let ready = MiniAppReadyRelease {
        miniapp_id: miniapp_id.into(),
        release: MiniAppReleaseRef {
            release_id: release_id.into(),
            artifact_id: artifact_payload.artifact_id.clone(),
            release_digest: artifact_payload.artifact_digest.clone(),
            manifest_digest: artifact_payload.manifest.payload_digest.clone(),
        },
        origin_operation_id: operation_id.into(),
        origin: MiniAppReadyOrigin::Build,
        source_lineage: MiniAppSourceLineage::Managed {
            project_id: project_id.into(),
            source_snapshot_digest: source_digest.into(),
            dependency_lock_digest: lock_digest.into(),
            build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
            build_generation: 1,
        },
        matching_service_test_receipt: None,
        created_at_ms: created_at,
    };
    ready.validate_for_artifact(&artifact_payload).unwrap();
    let release_record_json = String::from_utf8(
        nomifun_agent_contracts::canonical_json_bytes(&ready).unwrap(),
    )
    .unwrap();
    MiniAppReleaseRow {
        id: 0,
        release_id: release_id.to_owned(),
        miniapp_id: miniapp_id.to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_digest: artifact.artifact_digest.clone(),
        manifest_digest: artifact.manifest_digest.clone(),
        release_digest: artifact.artifact_digest.clone(),
        origin_kind: "build".to_owned(),
        origin_operation_id: operation_id.to_owned(),
        source_kind: "managed".to_owned(),
        project_id: Some(project_id.to_owned()),
        source_snapshot_digest: Some(source_digest.to_owned()),
        dependency_lock_digest: Some(lock_digest.to_owned()),
        build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.to_owned()),
        build_generation: Some(1),
        release_record_json,
        created_at,
    }
}

fn set_release_build_generation(release: &mut MiniAppReleaseRow, build_generation: i64) {
    let mut record: MiniAppReadyRelease =
        serde_json::from_str(&release.release_record_json).unwrap();
    if let MiniAppSourceLineage::Managed {
        build_generation: record_generation,
        ..
    } = &mut record.source_lineage
    {
        *record_generation = build_generation as u64;
    } else {
        panic!("repository fixture must use managed source lineage");
    }
    release.build_generation = Some(build_generation);
    release.release_record_json = String::from_utf8(
        nomifun_agent_contracts::canonical_json_bytes(&record).unwrap(),
    )
    .unwrap();
}

async fn create_editable_app(
    repository: &SqliteMiniAppM1Repository,
    owner: &str,
) -> MiniAppM1Snapshot {
    repository
        .create_with_source(&CreateMiniAppM1WithSourceParams {
            create: create_params(
                owner,
                MINIAPP_ID,
                PROJECT_ID,
                0,
                MiniAppM1Kind::UiOnly,
                10,
            ),
            source: managed_source("sources/owner/project/source", 'b', 'c', 1),
        })
        .await
        .unwrap()
}

async fn create_enabled_service_app(
    repository: &SqliteMiniAppM1Repository,
    owner: &str,
) -> MiniAppM1Snapshot {
    let ready = create_ready_service_app(repository, owner).await;
    let release_id = ready.product.ready_release_id.clone().unwrap();
    let artifact_digest = ready.product.ready_release_digest.clone().unwrap();
    let published = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.to_owned(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: ready.product.product_revision,
            expected_pointer_revision: ready.product.pointer_revision,
            expected_active_release_epoch: 0,
            expected_ready_release_id: release_id,
            expected_ready_release_digest: artifact_digest,
            expected_active_release_digest: None,
            target_catalog_digest: "d".repeat(64),
            auto_publish_guard: None,
            updated_at: 24,
        })
        .await
        .unwrap();
    repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.to_owned(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: published.product.product_revision,
            expected_pointer_revision: published.product.pointer_revision,
            expected_active_release_digest: published.product.active_release_digest.clone(),
            enabled: true,
            updated_at: 25,
        })
        .await
        .unwrap()
}

async fn create_ready_service_app(
    repository: &SqliteMiniAppM1Repository,
    owner: &str,
) -> MiniAppM1Snapshot {
    let source = managed_source("sources/owner/service/source", 'b', 'c', 1);
    repository
        .create_with_source(&CreateMiniAppM1WithSourceParams {
            create: create_params(
                owner,
                SERVICE_MINIAPP_ID,
                SERVICE_PROJECT_ID,
                0,
                MiniAppM1Kind::Service,
                10,
            ),
            source: source.clone(),
        })
        .await
        .unwrap();
    let operation_id = Uuid::now_v7().to_string();
    start_build(
        repository,
        owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        1,
        &source,
        &operation_id,
        20,
    )
    .await;
    let artifact = service_artifact(
        owner,
        &Uuid::now_v7().to_string(),
        "delete-service",
        21,
        false,
        false,
        false,
    );
    let artifact_digest = artifact.artifact_digest.clone();
    let release_id = Uuid::now_v7().to_string();
    let release = release(
        owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        &artifact,
        &release_id,
        &artifact_digest,
        &operation_id,
        &source.source_head_digest,
        &source.dependency_lock_digest,
        22,
    );
    repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.to_owned(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            project_id: SERVICE_PROJECT_ID.to_owned(),
            operation_id: operation_id.clone(),
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact,
            release,
            bounded_log_tail: Vec::new(),
            finished_at_ms: 23,
        })
        .await
        .unwrap()
}

fn service_test_receipt_params(
    snapshot: &MiniAppM1Snapshot,
    receipt_id: String,
    digest_seed: char,
    issued_at_ms: i64,
) -> RecordMiniAppM1ServiceTestReceiptParams {
    let ready_row = snapshot.ready_release.as_ref().unwrap();
    let ready: MiniAppReadyRelease =
        serde_json::from_str(&ready_row.release_record_json).unwrap();
    let runtime = MiniAppServiceRuntimeFingerprint {
        runtime_installation_id: RuntimeInstallationId::from("test-runtime"),
        runtime_target: RuntimeTarget::from("windows-x86_64"),
        runtime_executable_digest: DigestHex::from(digest_seed.to_string().repeat(64)),
        node_version: VersionString::from("24.1.0"),
    };
    let receipt = MiniAppServiceTestReceipt {
        receipt_id: receipt_id.clone().into(),
        miniapp_id: SERVICE_MINIAPP_ID.into(),
        release: ready.release.clone(),
        service_run_key: DigestHex::from(
            char::from_u32(digest_seed as u32 + 1)
                .unwrap()
                .to_string()
                .repeat(64),
        ),
        outcome: MiniAppServiceTestOutcome::Passed,
        error_code: None,
        runtime: runtime.clone(),
        host_target: runtime.runtime_target.clone(),
        host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
        sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
        test_contract_version: MINIAPP_SERVICE_TEST_CONTRACT_VERSION.into(),
        resolved_test_input_digest: DigestHex::from(
            char::from_u32(digest_seed as u32 + 2)
                .unwrap()
                .to_string()
                .repeat(64),
        ),
        copied_kv_digest: DigestHex::from(
            char::from_u32(digest_seed as u32 + 3)
                .unwrap()
                .to_string()
                .repeat(64),
        ),
        copied_private_database_digest: None,
        empty_files_dir: None,
        migration_ledger_digest: None,
        credential_mode: MiniAppServiceTestCredentialMode::None,
        host_generation: 1,
        issued_at_ms,
    };
    RecordMiniAppM1ServiceTestReceiptParams {
        owner_user_id: snapshot.product.owner_user_id.clone(),
        miniapp_id: snapshot.product.miniapp_id.clone(),
        expected_product_revision: snapshot.product.product_revision,
        expected_pointer_revision: snapshot.product.pointer_revision,
        expected_config_revision: snapshot.product.config_revision,
        expected_credential_bindings_revision: snapshot.product.credential_bindings_revision,
        expected_ready_release_id: ready_row.release_id.clone(),
        expected_ready_release_digest: ready_row.release_digest.clone(),
        receipt_id,
        service_run_key: receipt.service_run_key.as_ref().to_owned(),
        outcome: receipt.outcome,
        error_code: None,
        receipt_digest: digest_payload(&receipt).unwrap().as_ref().to_owned(),
        runtime_fingerprint_digest: digest_payload(&runtime).unwrap().as_ref().to_owned(),
        resolved_test_input_digest: receipt.resolved_test_input_digest.as_ref().to_owned(),
        receipt: serde_json::to_value(receipt).unwrap(),
        issued_at_ms,
    }
}

#[tokio::test]
async fn service_test_receipt_is_owner_scoped_exact_and_retest_preserves_history() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let other_owner = insert_other_owner(database.pool()).await;
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let ready = create_ready_service_app(&repository, &owner).await;
    let original_pointer_revision = ready.product.pointer_revision;
    let first_receipt_id = Uuid::now_v7().to_string();
    let first_params =
        service_test_receipt_params(&ready, first_receipt_id.clone(), '1', 24);

    let mut wrong_owner_params =
        service_test_receipt_params(&ready, Uuid::now_v7().to_string(), '1', 24);
    wrong_owner_params.owner_user_id = other_owner.clone();
    let wrong_owner = repository
        .record_service_test_receipt_cas(&wrong_owner_params)
        .await
        .unwrap_err();
    assert!(wrong_owner.to_string().contains("not found"));

    let first = repository
        .record_service_test_receipt_cas(&first_params)
        .await
        .unwrap();
    assert_eq!(
        first.product.product_revision,
        ready.product.product_revision + 1
    );
    assert_eq!(first.product.pointer_revision, original_pointer_revision);
    assert_eq!(first.library_revision, ready.library_revision + 1);
    let first_row = repository
        .get_ready_service_test_receipt(&owner, SERVICE_MINIAPP_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_row.receipt_id, first_receipt_id);
    assert_eq!(first_row.outcome, "passed");
    assert_eq!(first_row.error_code, None);
    assert_eq!(
        first_row.resolved_test_input_digest,
        first_params.resolved_test_input_digest
    );
    assert!(
        repository
            .get_ready_service_test_receipt(&other_owner, SERVICE_MINIAPP_ID)
            .await
            .unwrap()
            .is_none()
    );
    let stored_ready: MiniAppReadyRelease =
        serde_json::from_str(&first.ready_release.as_ref().unwrap().release_record_json).unwrap();
    assert_eq!(
        stored_ready
            .matching_service_test_receipt
            .as_ref()
            .unwrap()
            .receipt_id
            .as_ref(),
        first_row.receipt_id
    );

    let mut stale =
        service_test_receipt_params(&first, Uuid::now_v7().to_string(), '4', 25);
    stale.expected_product_revision -= 1;
    let stale_error = repository
        .record_service_test_receipt_cas(&stale)
        .await
        .unwrap_err();
    assert!(stale_error.to_string().contains("CAS"));

    let second_receipt_id = Uuid::now_v7().to_string();
    let second_params =
        service_test_receipt_params(&first, second_receipt_id.clone(), '4', 25);
    let second = repository
        .record_service_test_receipt_cas(&second_params)
        .await
        .unwrap();
    assert_eq!(second.product.pointer_revision, original_pointer_revision);
    assert_eq!(second.product.product_revision, first.product.product_revision + 1);
    let current = repository
        .get_ready_service_test_receipt(&owner, SERVICE_MINIAPP_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.receipt_id, second_receipt_id);
    let history: Vec<String> = sqlx::query_scalar(
        "SELECT receipt_id FROM miniapp_service_test_receipts
         WHERE owner_user_id = ? AND miniapp_id = ? ORDER BY id",
    )
    .bind(&owner)
    .bind(SERVICE_MINIAPP_ID)
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert_eq!(history, [first_row.receipt_id, current.receipt_id]);

    repository
        .update_config_cas(
            &owner,
            SERVICE_MINIAPP_ID,
            second.product.product_revision,
            second.product.pointer_revision,
            second.product.config_revision,
            &second.product.config_schema_json,
            r#"{"mode":"changed"}"#,
            26,
        )
        .await
        .unwrap();
    assert!(
        repository
            .get_ready_service_test_receipt(&owner, SERVICE_MINIAPP_ID)
            .await
            .unwrap()
            .is_none(),
        "config mutation must stale the current receipt without deleting history"
    );
    let history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM miniapp_service_test_receipts
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(&owner)
    .bind(SERVICE_MINIAPP_ID)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(history_count, 2);
}

#[tokio::test]
async fn permanent_delete_removes_service_test_receipt_history() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let ready = create_ready_service_app(&repository, &owner).await;
    let tested = repository
        .record_service_test_receipt_cas(&service_test_receipt_params(
            &ready,
            Uuid::now_v7().to_string(),
            '1',
            24,
        ))
        .await
        .unwrap();
    let trashed = repository
        .trash_cas(&TrashMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: tested.product.product_revision,
            expected_pointer_revision: tested.product.pointer_revision,
            expected_active_release_digest: None,
            updated_at: 25,
        })
        .await
        .unwrap();
    let operation_id = Uuid::now_v7().to_string();
    repository
        .begin_delete(&BeginMiniAppM1DeleteParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: trashed.product.product_revision,
            expected_pointer_revision: trashed.product.pointer_revision,
            expected_active_release_digest: None,
            operation_id: operation_id.clone(),
            started_at_ms: 26,
        })
        .await
        .unwrap();
    repository
        .finalize_delete(&FinalizeMiniAppM1DeleteParams {
            owner_user_id: owner,
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            operation_id,
            expected_operation_revision: 1,
            finished_at_ms: 27,
        })
        .await
        .unwrap();
    let receipt_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_service_test_receipts")
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(receipt_count, 0);
}

#[tokio::test]
async fn trash_and_restore_are_exact_owner_scoped_lifecycle_transactions() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let other_owner = insert_other_owner(database.pool()).await;
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let enabled = create_enabled_service_app(&repository, &owner).await;
    let active_release_id = enabled.product.active_release_id.clone().unwrap();
    let active_release_digest = enabled.product.active_release_digest.clone().unwrap();
    repository
        .open_surface_session_cas(&OpenMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            surface_session_id: Uuid::now_v7().to_string(),
            capability_digest: "7".repeat(64),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_id: active_release_id,
            expected_active_release_digest: active_release_digest.clone(),
            expected_active_release_epoch: enabled.product.active_release_epoch,
            issued_at_ms: 26,
        })
        .await
        .unwrap();

    let cross_owner = repository
        .trash_cas(&TrashMiniAppM1Params {
            owner_user_id: other_owner,
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_digest: Some(active_release_digest.clone()),
            updated_at: 27,
        })
        .await
        .unwrap_err();
    assert!(cross_owner.to_string().contains("not found"));

    let stale = repository
        .trash_cas(&TrashMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: enabled.product.product_revision - 1,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_digest: Some(active_release_digest.clone()),
            updated_at: 27,
        })
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("CAS"));

    let trashed = repository
        .trash_cas(&TrashMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_digest: Some(active_release_digest),
            updated_at: 27,
        })
        .await
        .unwrap();
    assert_eq!(trashed.product.lifecycle, "trashed");
    assert_eq!(
        trashed.product.product_revision,
        enabled.product.product_revision + 1
    );
    assert!(trashed.catalog_publication.is_none());
    let surfaces: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM miniapp_surface_sessions WHERE miniapp_id = ?",
    )
    .bind(SERVICE_MINIAPP_ID)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(surfaces, 0);

    let restored = repository
        .restore_cas(&RestoreMiniAppM1Params {
            owner_user_id: owner,
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: trashed.product.product_revision,
            expected_pointer_revision: trashed.product.pointer_revision,
            expected_lifecycle: "trashed".to_owned(),
            updated_at: 28,
        })
        .await
        .unwrap();
    assert_eq!(restored.product.lifecycle, "disabled");
    assert_eq!(
        restored.product.product_revision,
        trashed.product.product_revision + 1
    );
    assert!(restored.catalog_publication.is_none());
}

#[tokio::test]
async fn deletion_intent_failure_restart_and_finalize_preserve_operation_history() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let other_owner = insert_other_owner(database.pool()).await;
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let enabled = create_enabled_service_app(&repository, &owner).await;
    repository
        .put_kv_cas(
            &owner,
            SERVICE_MINIAPP_ID,
            "service",
            "retained",
            &json!({"value": 1}),
            None,
            26,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_credential_bindings (
            miniapp_id, owner_user_id, slot_key, credential_id, created_at, updated_at
         ) VALUES (?, ?, 'token', 'credential-reference', 26, 26)",
    )
    .bind(SERVICE_MINIAPP_ID)
    .bind(&owner)
    .execute(database.pool())
    .await
    .unwrap();
    let trashed = repository
        .trash_cas(&TrashMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_digest: enabled.product.active_release_digest.clone(),
            updated_at: 27,
        })
        .await
        .unwrap();
    let first_operation_id = Uuid::now_v7().to_string();
    let deleting = repository
        .begin_delete(&BeginMiniAppM1DeleteParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: trashed.product.product_revision,
            expected_pointer_revision: trashed.product.pointer_revision,
            expected_active_release_digest: trashed.product.active_release_digest.clone(),
            operation_id: first_operation_id.clone(),
            started_at_ms: 28,
        })
        .await
        .unwrap();
    assert_eq!(deleting.product.lifecycle, "deleting");
    let first = repository
        .get_miniapp_operation(&owner, SERVICE_MINIAPP_ID, &first_operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.kind, "miniapp_permanent_delete");
    assert_eq!(first.state, "running");
    assert_eq!(first.progress_percent, None);
    assert!(repository
        .get_miniapp_operation(&other_owner, SERVICE_MINIAPP_ID, &first_operation_id)
        .await
        .unwrap()
        .is_none());

    repository
        .fail_delete(&FailMiniAppM1DeleteParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            operation_id: first_operation_id.clone(),
            expected_operation_revision: 1,
            error_code: "MINIAPP_DELETE_STORAGE_FAILED".to_owned(),
            updated_at: 29,
        })
        .await
        .unwrap();
    let failed = repository
        .get_miniapp_operation(&owner, SERVICE_MINIAPP_ID, &first_operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, "failed");
    assert_eq!(
        failed.last_error_code.as_deref(),
        Some("MINIAPP_DELETE_STORAGE_FAILED")
    );

    let second_operation_id = Uuid::now_v7().to_string();
    repository
        .restart_delete(&RestartMiniAppM1DeleteParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_failed_operation_id: first_operation_id.clone(),
            new_operation_id: second_operation_id.clone(),
            started_at_ms: 30,
        })
        .await
        .unwrap();
    let operations = repository
        .list_miniapp_operations(&owner, SERVICE_MINIAPP_ID)
        .await
        .unwrap();
    assert!(operations.iter().any(|operation| {
        operation.operation_id == first_operation_id && operation.state == "failed"
    }));
    assert!(operations.iter().any(|operation| {
        operation.operation_id == second_operation_id && operation.state == "running"
    }));

    let wrong_owner_finalize = repository
        .finalize_delete(&FinalizeMiniAppM1DeleteParams {
            owner_user_id: other_owner,
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            operation_id: second_operation_id.clone(),
            expected_operation_revision: 1,
            finished_at_ms: 31,
        })
        .await
        .unwrap_err();
    assert!(wrong_owner_finalize.to_string().contains("not found"));

    sqlx::query(
        "DELETE FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ? AND namespace = 'service'",
    )
    .bind(&owner)
    .bind(SERVICE_MINIAPP_ID)
    .execute(database.pool())
    .await
    .unwrap();
    let revision = repository
        .finalize_delete(&FinalizeMiniAppM1DeleteParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            operation_id: second_operation_id.clone(),
            expected_operation_revision: 1,
            finished_at_ms: 31,
        })
        .await
        .unwrap();
    assert_eq!(revision, deleting.library_revision + 3);
    assert!(repository
        .get(&owner, SERVICE_MINIAPP_ID)
        .await
        .unwrap()
        .is_none());
    for table in [
        "miniapp_surface_sessions",
        "miniapp_catalog_publications",
        "miniapp_publish_authorizations",
        "miniapp_credential_bindings",
        "miniapp_kv",
        "miniapp_build_operation_lineage",
        "miniapp_service_test_receipts",
        "miniapp_projects",
        "miniapp_releases",
        "miniapp_release_artifacts",
        "miniapp_deletion_intents",
    ] {
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(count, 0, "{table} must be cleaned");
    }
    let first_history: String = sqlx::query_scalar(
        "SELECT state FROM product_operations WHERE operation_id = ?",
    )
    .bind(first_operation_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let second_history: String = sqlx::query_scalar(
        "SELECT state FROM product_operations WHERE operation_id = ?",
    )
    .bind(second_operation_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(first_history, "failed");
    assert_eq!(second_history, "succeeded");
}

#[tokio::test]
async fn deleting_snapshot_fails_closed_without_its_exact_intent() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let enabled = create_enabled_service_app(&repository, &owner).await;
    let trashed = repository
        .trash_cas(&TrashMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_digest: enabled.product.active_release_digest,
            updated_at: 27,
        })
        .await
        .unwrap();
    let operation_id = Uuid::now_v7().to_string();
    repository
        .begin_delete(&BeginMiniAppM1DeleteParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: trashed.product.product_revision,
            expected_pointer_revision: trashed.product.pointer_revision,
            expected_active_release_digest: trashed.product.active_release_digest,
            operation_id: operation_id.clone(),
            started_at_ms: 28,
        })
        .await
        .unwrap();
    sqlx::query(
        "DELETE FROM miniapp_deletion_intents
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(&owner)
    .bind(SERVICE_MINIAPP_ID)
    .execute(database.pool())
    .await
    .unwrap();
    let error = repository
        .get(&owner, SERVICE_MINIAPP_ID)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("without an exact deletion intent"));
    let library_error = repository.library(&owner).await.unwrap_err();
    assert!(library_error
        .to_string()
        .contains("without an exact deletion intent"));

    sqlx::query(
        "INSERT INTO miniapp_deletion_intents (
            miniapp_id, owner_user_id, operation_id, started_at_ms, last_error_code
         ) VALUES (?, ?, ?, 28, NULL)",
    )
    .bind(SERVICE_MINIAPP_ID)
    .bind(&owner)
    .bind(&operation_id)
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE miniapp_products SET lifecycle = 'trashed'
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(&owner)
    .bind(SERVICE_MINIAPP_ID)
    .execute(database.pool())
    .await
    .unwrap();
    let outside = repository
        .get(&owner, SERVICE_MINIAPP_ID)
        .await
        .unwrap_err();
    assert!(outside
        .to_string()
        .contains("intent exists outside the deleting lifecycle"));
}

#[tokio::test]
async fn service_product_supports_build_ready_publish_rollback_and_lifecycle() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let source = managed_source("sources/owner/service/source", 'b', 'c', 1);

    let created = repository
        .create_with_source(&CreateMiniAppM1WithSourceParams {
            create: create_params(
                &owner,
                SERVICE_MINIAPP_ID,
                SERVICE_PROJECT_ID,
                0,
                MiniAppM1Kind::Service,
                10,
            ),
            source: source.clone(),
        })
        .await
        .unwrap();
    assert_eq!(created.product.kind, MiniAppM1Kind::Service.as_str());
    assert_eq!(created.project.source_state, "editable");

    let operation_one = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        1,
        &source,
        &operation_one,
        20,
    )
    .await;
    let artifact_one = service_artifact(
        &owner,
        &Uuid::now_v7().to_string(),
        "service-one",
        21,
        false,
        false,
        false,
    );
    let digest_one = artifact_one.artifact_digest.clone();
    let release_one = release(
        &owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        &artifact_one,
        &Uuid::now_v7().to_string(),
        &digest_one,
        &operation_one,
        &source.source_head_digest,
        &source.dependency_lock_digest,
        22,
    );
    let ready_one = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            project_id: SERVICE_PROJECT_ID.to_owned(),
            operation_id: operation_one,
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: artifact_one,
            release: release_one,
            bounded_log_tail: vec!["service build succeeded".to_owned()],
            finished_at_ms: 23,
        })
        .await
        .unwrap();
    assert!(ready_one.ready_release.is_some());
    assert!(ready_one.active_release.is_none());

    let published_one = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: 2,
            expected_pointer_revision: 2,
            expected_active_release_epoch: 0,
            expected_ready_release_id: ready_one
                .product
                .ready_release_id
                .clone()
                .unwrap(),
            expected_ready_release_digest: ready_one
                .product
                .ready_release_digest
                .clone()
                .unwrap(),
            expected_active_release_digest: None,
            target_catalog_digest: "3".repeat(64),
            auto_publish_guard: None,
            updated_at: 24,
        })
        .await
        .unwrap();
    let active_one_id = published_one.product.active_release_id.clone().unwrap();
    let active_one_digest = published_one
        .product
        .active_release_digest
        .clone()
        .unwrap();
    assert_eq!(published_one.product.active_release_epoch, 1);

    let enabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: 3,
            expected_pointer_revision: 3,
            expected_active_release_digest: Some(active_one_digest.clone()),
            enabled: true,
            updated_at: 25,
        })
        .await
        .unwrap();
    assert_eq!(enabled.product.lifecycle, "enabled");
    assert!(enabled.catalog_publication.is_some());

    let operation_two = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        1,
        &source,
        &operation_two,
        30,
    )
    .await;
    let artifact_two = service_artifact(
        &owner,
        &Uuid::now_v7().to_string(),
        "service-two",
        31,
        false,
        false,
        false,
    );
    let digest_two = artifact_two.artifact_digest.clone();
    let release_two_id = Uuid::now_v7().to_string();
    let release_two = release(
        &owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        &artifact_two,
        &release_two_id,
        &digest_two,
        &operation_two,
        &source.source_head_digest,
        &source.dependency_lock_digest,
        32,
    );
    let ready_two = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            project_id: SERVICE_PROJECT_ID.to_owned(),
            operation_id: operation_two,
            expected_product_revision: 4,
            expected_pointer_revision: 3,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: artifact_two,
            release: release_two,
            bounded_log_tail: vec![],
            finished_at_ms: 33,
        })
        .await
        .unwrap();
    assert_eq!(
        ready_two.product.ready_release_digest,
        Some(digest_two.clone())
    );
    assert_eq!(
        ready_two.product.active_release_digest,
        Some(active_one_digest.clone())
    );
    let published_two = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: 5,
            expected_pointer_revision: 4,
            expected_active_release_epoch: 1,
            expected_ready_release_id: release_two_id,
            expected_ready_release_digest: digest_two.clone(),
            expected_active_release_digest: Some(active_one_digest.clone()),
            target_catalog_digest: "4".repeat(64),
            auto_publish_guard: None,
            updated_at: 34,
        })
        .await
        .unwrap();
    assert_eq!(published_two.product.previous_release_id, Some(active_one_id.clone()));
    assert_eq!(published_two.product.active_release_epoch, 2);

    let rolled_back = repository
        .rollback_previous_cas(&RollbackMiniAppM1PreviousParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: 6,
            expected_pointer_revision: 5,
            expected_active_release_epoch: 2,
            expected_current_release_id: published_two
                .product
                .active_release_id
                .clone()
                .unwrap(),
            expected_current_release_digest: published_two
                .product
                .active_release_digest
                .clone()
                .unwrap(),
            expected_previous_release_id: active_one_id,
            expected_previous_release_digest: active_one_digest.clone(),
            target_catalog_digest: "5".repeat(64),
            updated_at: 35,
        })
        .await
        .unwrap();
    assert_eq!(rolled_back.product.active_release_epoch, 3);
    assert_eq!(
        rolled_back.product.active_release_digest,
        Some(active_one_digest.clone())
    );

    let disabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner,
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            expected_product_revision: 7,
            expected_pointer_revision: 6,
            expected_active_release_digest: Some(active_one_digest),
            enabled: false,
            updated_at: 36,
        })
        .await
        .unwrap();
    assert_eq!(disabled.product.lifecycle, "disabled");
    assert!(disabled.catalog_publication.is_none());
}

#[tokio::test]
async fn service_product_accepts_managed_storage_and_rejects_wrong_manifest_kind() {
    for (index, (uses_files, uses_private_database, with_migration)) in
        [(true, false, false), (false, true, false), (false, true, true)]
            .into_iter()
            .enumerate()
    {
        let database = init_miniapp_test_database().await;
        let owner = installation_owner_id(database.pool()).await.unwrap();
        let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
        let source = managed_source("sources/owner/service/source", 'b', 'c', 1);
        repository
            .create_with_source(&CreateMiniAppM1WithSourceParams {
                create: create_params(
                    &owner,
                    SERVICE_MINIAPP_ID,
                    SERVICE_PROJECT_ID,
                    0,
                    MiniAppM1Kind::Service,
                    10,
                ),
                source: source.clone(),
            })
            .await
            .unwrap();
        let operation_id = Uuid::now_v7().to_string();
        start_build(
            &repository,
            &owner,
            SERVICE_MINIAPP_ID,
            SERVICE_PROJECT_ID,
            1,
            &source,
            &operation_id,
            20 + index as i64 * 3,
        )
        .await;
        let artifact = service_artifact(
            &owner,
            &Uuid::now_v7().to_string(),
            &format!("unsupported-{index}"),
            21 + index as i64 * 3,
            uses_files,
            uses_private_database,
            with_migration,
        );
        let release = release(
            &owner,
            SERVICE_MINIAPP_ID,
            SERVICE_PROJECT_ID,
            &artifact,
            &Uuid::now_v7().to_string(),
            &artifact.artifact_digest,
            &operation_id,
            &source.source_head_digest,
            &source.dependency_lock_digest,
            22 + index as i64 * 3,
        );
        let ready = repository
            .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
                owner_user_id: owner.clone(),
                miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
                project_id: SERVICE_PROJECT_ID.to_owned(),
                operation_id: operation_id.clone(),
                expected_product_revision: 1,
                expected_pointer_revision: 1,
                expected_project_revision: 1,
                expected_build_generation: 1,
                artifact,
                release,
                bounded_log_tail: vec![],
                finished_at_ms: 23 + index as i64 * 3,
            })
            .await
            .unwrap();
        assert_eq!(
            ready
                .ready_release
                .as_ref()
                .map(|release| release.release_digest.as_str()),
            ready.product.ready_release_digest.as_deref()
        );

        assert_eq!(ready.product.kind, "service");
    }

    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let source = managed_source("sources/owner/service/source", 'b', 'c', 1);
    repository
        .create_with_source(&CreateMiniAppM1WithSourceParams {
            create: create_params(
                &owner,
                SERVICE_MINIAPP_ID,
                SERVICE_PROJECT_ID,
                0,
                MiniAppM1Kind::Service,
                10,
            ),
            source: source.clone(),
        })
        .await
        .unwrap();
    let operation_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        1,
        &source,
        &operation_id,
        40,
    )
    .await;
    let ui_artifact = artifact(
        &owner,
        &Uuid::now_v7().to_string(),
        "ui-for-service",
        "ignored",
        41,
    );
    let ui_release = release(
        &owner,
        SERVICE_MINIAPP_ID,
        SERVICE_PROJECT_ID,
        &ui_artifact,
        &Uuid::now_v7().to_string(),
        &ui_artifact.artifact_digest,
        &operation_id,
        &source.source_head_digest,
        &source.dependency_lock_digest,
        42,
    );
    let error = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: SERVICE_MINIAPP_ID.to_owned(),
            project_id: SERVICE_PROJECT_ID.to_owned(),
            operation_id: operation_id.clone(),
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: ui_artifact,
            release: ui_release,
            bounded_log_tail: vec![],
            finished_at_ms: 43,
        })
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Service MiniApp Release must declare service/main.mjs"));
}

#[tokio::test]
async fn ready_release_and_pointer_cas_bind_exact_lineage_and_owner() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let created = create_editable_app(&repository, &owner).await;
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);
    assert_eq!(
        created.project.build_profile_version.as_deref(),
        Some(MINIAPP_RELEASE_PROFILE_VERSION)
    );

    let op_one = Uuid::now_v7().to_string();
    let artifact_one_id = Uuid::now_v7().to_string();
    let release_one_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &op_one,
        30,
    )
    .await;
    let artifact_one = artifact(
        &owner,
        &artifact_one_id,
        &"d".repeat(64),
        &"e".repeat(64),
        31,
    );
    let digest_one = artifact_one.artifact_digest.clone();
    let release_one = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_one,
        &release_one_id,
        &digest_one,
        &op_one,
        &"b".repeat(64),
        &"c".repeat(64),
        32,
    );
    let ready_one = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: op_one.clone(),
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: artifact_one.clone(),
            release: release_one.clone(),
            bounded_log_tail: vec!["build started".into(), "build succeeded".into()],
            finished_at_ms: 33,
        })
        .await
        .unwrap();
    assert_eq!(
        ready_one.product.ready_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(ready_one.library_revision, 2);
    let succeeded = repository
        .get_build_operation(&owner, MINIAPP_ID, &op_one)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(succeeded.state, "succeeded");
    assert_eq!(succeeded.progress_percent, Some(100));

    let published_one = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 2,
            expected_pointer_revision: 2,
            expected_active_release_epoch: 0,
            expected_ready_release_id: release_one_id.clone(),
            expected_ready_release_digest: digest_one.clone(),
            expected_active_release_digest: None,
            target_catalog_digest: "3".repeat(64),
            auto_publish_guard: None,
            updated_at: 34,
        })
        .await
        .unwrap();
    assert_eq!(
        published_one.product.active_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert!(published_one.product.ready_release_id.is_none());
    assert!(published_one.catalog_publication.is_none());
    assert_eq!(published_one.product.active_release_epoch, 1);

    let op_two = Uuid::now_v7().to_string();
    let artifact_two_id = Uuid::now_v7().to_string();
    let release_two_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &op_two,
        40,
    )
    .await;
    let artifact_two = artifact(
        &owner,
        &artifact_two_id,
        &"1".repeat(64),
        &"2".repeat(64),
        41,
    );
    let digest_two = artifact_two.artifact_digest.clone();
    let release_two = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_two,
        &release_two_id,
        &digest_two,
        &op_two,
        &"b".repeat(64),
        &"c".repeat(64),
        42,
    );
    let _ready_two = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: op_two,
            expected_product_revision: 3,
            expected_pointer_revision: 3,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: artifact_two,
            release: release_two,
            bounded_log_tail: vec!["build started".into(), "build succeeded".into()],
            finished_at_ms: 43,
        })
        .await
        .unwrap();

    let activated = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 4,
            expected_pointer_revision: 4,
            expected_active_release_epoch: 1,
            expected_ready_release_id: release_two_id.clone(),
            expected_ready_release_digest: digest_two.clone(),
            expected_active_release_digest: Some(digest_one.clone()),
            target_catalog_digest: "4".repeat(64),
            auto_publish_guard: None,
            updated_at: 44,
        })
        .await
        .unwrap();
    assert_eq!(
        activated.product.active_release_id.as_deref(),
        Some(release_two_id.as_str())
    );
    assert_eq!(
        activated.product.previous_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(activated.product.active_release_epoch, 2);

    let stale = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 4,
            expected_pointer_revision: 4,
            expected_active_release_epoch: 1,
            expected_ready_release_id: release_two_id.clone(),
            expected_ready_release_digest: digest_two.clone(),
            expected_active_release_digest: Some(digest_one.clone()),
            target_catalog_digest: "5".repeat(64),
            auto_publish_guard: None,
            updated_at: 45,
        })
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("CAS"));

    let rolled_back = repository
        .rollback_previous_cas(&RollbackMiniAppM1PreviousParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 5,
            expected_pointer_revision: 5,
            expected_active_release_epoch: 2,
            expected_current_release_id: release_two_id.clone(),
            expected_current_release_digest: digest_two.clone(),
            expected_previous_release_id: release_one_id.clone(),
            expected_previous_release_digest: digest_one.clone(),
            target_catalog_digest: "5".repeat(64),
            updated_at: 46,
        })
        .await
        .unwrap();
    assert_eq!(
        rolled_back.product.active_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(rolled_back.product.active_release_epoch, 3);
    assert_eq!(
        rolled_back.product.previous_release_digest.as_deref(),
        Some(digest_two.as_str())
    );

    let enabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 6,
            expected_pointer_revision: 6,
            expected_active_release_digest: Some(digest_one.clone()),
            enabled: true,
            updated_at: 47,
        })
        .await
        .unwrap();
    let catalog = enabled.catalog_publication.as_ref().unwrap();
    assert_eq!(catalog.active_release_id, release_one_id);
    assert_eq!(catalog.active_release_epoch, 3);

    sqlx::query(
        "CREATE TRIGGER fail_miniapp_catalog_update
         BEFORE UPDATE ON miniapp_catalog_publications
         BEGIN
             SELECT RAISE(ABORT, 'forced catalog projection failure');
         END",
    )
    .execute(database.pool())
    .await
    .unwrap();
    let failed_catalog_cutover = repository
        .rollback_previous_cas(&RollbackMiniAppM1PreviousParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 7,
            expected_pointer_revision: 6,
            expected_active_release_epoch: 3,
            expected_current_release_id: release_one_id.clone(),
            expected_current_release_digest: digest_one.clone(),
            expected_previous_release_id: release_two_id,
            expected_previous_release_digest: digest_two.clone(),
            target_catalog_digest: "6".repeat(64),
            updated_at: 48,
        })
        .await
        .unwrap_err();
    assert!(
        failed_catalog_cutover
            .to_string()
            .contains("forced catalog projection failure")
    );
    let after_failed_catalog_cutover = repository
        .get(&owner, MINIAPP_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_failed_catalog_cutover.product.product_revision, 7);
    assert_eq!(after_failed_catalog_cutover.product.pointer_revision, 6);
    assert_eq!(after_failed_catalog_cutover.product.active_release_epoch, 3);
    assert_eq!(
        after_failed_catalog_cutover
            .product
            .active_release_id
            .as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(after_failed_catalog_cutover.library_revision, 7);
    assert_eq!(
        after_failed_catalog_cutover.catalog_publication,
        enabled.catalog_publication
    );
    sqlx::query("DROP TRIGGER fail_miniapp_catalog_update")
        .execute(database.pool())
        .await
        .unwrap();

    let authorization_id = Uuid::now_v7().to_string();
    let auto = repository
        .set_auto_publish_cas(&SetMiniAppM1AutoPublishParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 7,
            expected_pointer_revision: 6,
            expected_authorization_revision: None,
            authorization_id: authorization_id.clone(),
            enabled: true,
            user_authorized_at_ms: 48,
            updated_at: 48,
        })
        .await
        .unwrap();
    assert!(auto.auto_publish_authorization.as_ref().unwrap().enabled);

    let disabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 8,
            expected_pointer_revision: 6,
            expected_active_release_digest: Some(digest_one.clone()),
            enabled: false,
            updated_at: 49,
        })
        .await
        .unwrap();
    assert!(disabled.catalog_publication.is_none());

    let manual = repository
        .set_auto_publish_cas(&SetMiniAppM1AutoPublishParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 9,
            expected_pointer_revision: 6,
            expected_authorization_revision: Some(1),
            authorization_id,
            enabled: false,
            user_authorized_at_ms: 48,
            updated_at: 50,
        })
        .await
        .unwrap();
    let authorization = manual.auto_publish_authorization.unwrap();
    assert!(!authorization.enabled);
    assert_eq!(authorization.revision, 2);
}

#[tokio::test]
async fn surface_sessions_are_exact_revocable_and_disable_enable_aba_safe() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);
    let operation_id = Uuid::now_v7().to_string();
    let artifact_id = Uuid::now_v7().to_string();
    let release_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &operation_id,
        20,
    )
    .await;
    let artifact = artifact(
        &owner,
        &artifact_id,
        &"d".repeat(64),
        &"e".repeat(64),
        21,
    );
    let digest = artifact.artifact_digest.clone();
    let release = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact,
        &release_id,
        &digest,
        &operation_id,
        &"b".repeat(64),
        &"c".repeat(64),
        22,
    );
    let ready = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id,
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact,
            release,
            bounded_log_tail: vec![],
            finished_at_ms: 23,
        })
        .await
        .unwrap();
    let published = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: ready.product.product_revision,
            expected_pointer_revision: ready.product.pointer_revision,
            expected_active_release_epoch: 0,
            expected_ready_release_id: release_id.clone(),
            expected_ready_release_digest: digest.clone(),
            expected_active_release_digest: None,
            target_catalog_digest: "1".repeat(64),
            auto_publish_guard: None,
            updated_at: 24,
        })
        .await
        .unwrap();
    let enabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: published.product.product_revision,
            expected_pointer_revision: published.product.pointer_revision,
            expected_active_release_digest: Some(digest.clone()),
            enabled: true,
            updated_at: 25,
        })
        .await
        .unwrap();

    let first_session_id = Uuid::now_v7().to_string();
    let first = repository
        .open_surface_session_cas(&OpenMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: first_session_id.clone(),
            capability_digest: "2".repeat(64),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_id: release_id.clone(),
            expected_active_release_digest: digest.clone(),
            expected_active_release_epoch: 1,
            issued_at_ms: 26,
        })
        .await
        .unwrap();
    let written = repository
        .execute_surface_kv(&ExecuteMiniAppM1SurfaceKvParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: first.surface_session_id.clone(),
            expected_surface_generation: first.generation,
            expected_capability_digest: first.capability_digest.clone(),
            expected_active_release_epoch: 1,
            expected_active_release_digest: digest.clone(),
            namespace: "surface".to_owned(),
            key: "state".to_owned(),
            operation: MiniAppM1SurfaceKvOperation::Set {
                value: json!({"value": 1}),
            },
            updated_at: 27,
        })
        .await
        .unwrap();
    assert_eq!(written, MiniAppM1SurfaceKvResult::Written { revision: 1 });

    let second_session_id = Uuid::now_v7().to_string();
    let second = repository
        .open_surface_session_cas(&OpenMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: second_session_id.clone(),
            capability_digest: "3".repeat(64),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_id: release_id.clone(),
            expected_active_release_digest: digest.clone(),
            expected_active_release_epoch: 1,
            issued_at_ms: 28,
        })
        .await
        .unwrap();
    assert_eq!(second.generation, first.generation + 1);
    assert!(repository
        .execute_surface_kv(&ExecuteMiniAppM1SurfaceKvParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: first.surface_session_id,
            expected_surface_generation: first.generation,
            expected_capability_digest: first.capability_digest,
            expected_active_release_epoch: 1,
            expected_active_release_digest: digest.clone(),
            namespace: "surface".to_owned(),
            key: "state".to_owned(),
            operation: MiniAppM1SurfaceKvOperation::Set {
                value: json!({"value": "stale"}),
            },
            updated_at: 29,
        })
        .await
        .is_err());
    assert!(repository
        .resolve_surface_session(&ResolveMiniAppM1SurfaceSessionParams {
            miniapp_id: MINIAPP_ID.to_owned(),
            capability_digest: "2".repeat(64),
            expected_active_release_digest: digest.clone(),
            expected_active_release_epoch: 1,
        })
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .close_surface_session_cas(&CloseMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: second.surface_session_id.clone(),
            capability_digest: second.capability_digest.clone(),
        })
        .await
        .unwrap());
    assert!(!repository
        .close_surface_session_cas(&CloseMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: second.surface_session_id,
            capability_digest: second.capability_digest,
        })
        .await
        .unwrap());

    let third = repository
        .open_surface_session_cas(&OpenMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: Uuid::now_v7().to_string(),
            capability_digest: "4".repeat(64),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_id: release_id,
            expected_active_release_digest: digest.clone(),
            expected_active_release_epoch: 1,
            issued_at_ms: 30,
        })
        .await
        .unwrap();
    let disabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: enabled.product.product_revision,
            expected_pointer_revision: enabled.product.pointer_revision,
            expected_active_release_digest: Some(digest.clone()),
            enabled: false,
            updated_at: 31,
        })
        .await
        .unwrap();
    let reenabled = repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: disabled.product.product_revision,
            expected_pointer_revision: disabled.product.pointer_revision,
            expected_active_release_digest: Some(digest.clone()),
            enabled: true,
            updated_at: 32,
        })
        .await
        .unwrap();
    assert!(repository
        .execute_surface_kv(&ExecuteMiniAppM1SurfaceKvParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: third.surface_session_id,
            expected_surface_generation: third.generation,
            expected_capability_digest: third.capability_digest,
            expected_active_release_epoch: 1,
            expected_active_release_digest: digest.clone(),
            namespace: "surface".to_owned(),
            key: "state".to_owned(),
            operation: MiniAppM1SurfaceKvOperation::Get,
            updated_at: 33,
        })
        .await
        .is_err());
    let restarted_session = repository
        .open_surface_session_cas(&OpenMiniAppM1SurfaceSessionParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            surface_session_id: Uuid::now_v7().to_string(),
            capability_digest: "5".repeat(64),
            expected_product_revision: reenabled.product.product_revision,
            expected_pointer_revision: reenabled.product.pointer_revision,
            expected_active_release_id: reenabled
                .product
                .active_release_id
                .clone()
                .unwrap(),
            expected_active_release_digest: digest.clone(),
            expected_active_release_epoch: 1,
            issued_at_ms: 34,
        })
        .await
        .unwrap();
    assert_eq!(
        repository
            .revoke_all_surface_sessions_on_startup()
            .await
            .unwrap(),
        1
    );
    assert!(repository
        .resolve_surface_session(&ResolveMiniAppM1SurfaceSessionParams {
            miniapp_id: MINIAPP_ID.to_owned(),
            capability_digest: restarted_session.capability_digest,
            expected_active_release_digest: digest.clone(),
            expected_active_release_epoch: 1,
        })
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn identical_artifact_content_keeps_distinct_release_lineage_and_pointer_identity() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source_one = managed_source("sources/owner/project/source", 'b', 'c', 1);
    let operation_one = Uuid::now_v7().to_string();
    let artifact_id = Uuid::now_v7().to_string();
    let release_one_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source_one,
        &operation_one,
        20,
    )
    .await;
    let artifact_one = artifact(
        &owner,
        &artifact_id,
        &"d".repeat(64),
        &"e".repeat(64),
        21,
    );
    let digest = artifact_one.artifact_digest.clone();
    let release_one = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_one,
        &release_one_id,
        &digest,
        &operation_one,
        &"b".repeat(64),
        &"c".repeat(64),
        22,
    );
    let ready_one = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: operation_one,
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: artifact_one.clone(),
            release: release_one,
            bounded_log_tail: vec![],
            finished_at_ms: 23,
        })
        .await
        .unwrap();
    let published_one = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: ready_one.product.product_revision,
            expected_pointer_revision: ready_one.product.pointer_revision,
            expected_active_release_epoch: 0,
            expected_ready_release_id: release_one_id.clone(),
            expected_ready_release_digest: digest.clone(),
            expected_active_release_digest: None,
            target_catalog_digest: "1".repeat(64),
            auto_publish_guard: None,
            updated_at: 24,
        })
        .await
        .unwrap();

    let source_two = managed_source("sources/owner/project/source", 'f', 'c', 2);
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 1,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some(source_two.managed_source_path.clone()),
            source_head_digest: Some(source_two.source_head_digest.clone()),
            dependency_lock_digest: Some(source_two.dependency_lock_digest.clone()),
            build_profile_version: Some(source_two.build_profile_version.clone()),
            build_generation: 2,
            updated_at: 25,
        })
        .await
        .unwrap();
    let operation_two = Uuid::now_v7().to_string();
    let release_two_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        2,
        &source_two,
        &operation_two,
        26,
    )
    .await;
    let artifact_two = MiniAppReleaseArtifactRow {
        created_at: 27,
        ..artifact_one
    };
    let mut release_two = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_two,
        &release_two_id,
        &digest,
        &operation_two,
        &"f".repeat(64),
        &"c".repeat(64),
        28,
    );
    set_release_build_generation(&mut release_two, 2);
    let ready_two = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: operation_two,
            expected_product_revision: published_one.product.product_revision,
            expected_pointer_revision: published_one.product.pointer_revision,
            expected_project_revision: 2,
            expected_build_generation: 2,
            artifact: artifact_two,
            release: release_two,
            bounded_log_tail: vec![],
            finished_at_ms: 29,
        })
        .await
        .unwrap();
    assert_eq!(
        ready_two.product.ready_release_digest,
        ready_two.product.active_release_digest
    );
    assert_ne!(
        ready_two.product.ready_release_id,
        ready_two.product.active_release_id
    );

    let published_two = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: ready_two.product.product_revision,
            expected_pointer_revision: ready_two.product.pointer_revision,
            expected_active_release_epoch: 1,
            expected_ready_release_id: release_two_id.clone(),
            expected_ready_release_digest: digest.clone(),
            expected_active_release_digest: Some(digest.clone()),
            target_catalog_digest: "2".repeat(64),
            auto_publish_guard: None,
            updated_at: 30,
        })
        .await
        .unwrap();
    assert_eq!(
        published_two.product.active_release_id.as_deref(),
        Some(release_two_id.as_str())
    );
    assert_eq!(
        published_two.product.previous_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(
        published_two.product.active_release_digest,
        published_two.product.previous_release_digest
    );
    assert_eq!(published_two.product.active_release_epoch, 2);

    let rolled_back = repository
        .rollback_previous_cas(&RollbackMiniAppM1PreviousParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: published_two.product.product_revision,
            expected_pointer_revision: published_two.product.pointer_revision,
            expected_active_release_epoch: 2,
            expected_current_release_id: release_two_id,
            expected_current_release_digest: digest.clone(),
            expected_previous_release_id: release_one_id.clone(),
            expected_previous_release_digest: digest.clone(),
            target_catalog_digest: "1".repeat(64),
            updated_at: 31,
        })
        .await
        .unwrap();
    assert_eq!(
        rolled_back.product.active_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(rolled_back.product.active_release_epoch, 3);
    let artifact_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_release_artifacts")
            .fetch_one(database.pool())
            .await
            .unwrap();
    let release_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_releases")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(artifact_count, 1);
    assert_eq!(release_count, 2);
}

#[tokio::test]
async fn auto_publish_guard_rejects_advanced_source_and_revoke_wins_running_build() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source_one = managed_source("sources/owner/project/source", 'b', 'c', 1);
    let operation_one = Uuid::now_v7().to_string();
    let artifact_one_id = Uuid::now_v7().to_string();
    let release_one_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source_one,
        &operation_one,
        20,
    )
    .await;
    let artifact_one = artifact(
        &owner,
        &artifact_one_id,
        &"d".repeat(64),
        &"e".repeat(64),
        21,
    );
    let digest_one = artifact_one.artifact_digest.clone();
    let release_one = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_one,
        &release_one_id,
        &digest_one,
        &operation_one,
        &"b".repeat(64),
        &"c".repeat(64),
        22,
    );
    let ready_one = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: operation_one,
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact: artifact_one,
            release: release_one,
            bounded_log_tail: vec![],
            finished_at_ms: 23,
        })
        .await
        .unwrap();
    let published = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: ready_one.product.product_revision,
            expected_pointer_revision: ready_one.product.pointer_revision,
            expected_active_release_epoch: 0,
            expected_ready_release_id: release_one_id.clone(),
            expected_ready_release_digest: digest_one.clone(),
            expected_active_release_digest: None,
            target_catalog_digest: "1".repeat(64),
            auto_publish_guard: None,
            updated_at: 24,
        })
        .await
        .unwrap();
    let authorization_id = Uuid::now_v7().to_string();
    let authorized = repository
        .set_auto_publish_cas(&SetMiniAppM1AutoPublishParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: published.product.product_revision,
            expected_pointer_revision: published.product.pointer_revision,
            expected_authorization_revision: None,
            authorization_id: authorization_id.clone(),
            enabled: true,
            user_authorized_at_ms: 25,
            updated_at: 25,
        })
        .await
        .unwrap();
    let source_two = managed_source("sources/owner/project/source", 'f', 'c', 2);
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 1,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some(source_two.managed_source_path.clone()),
            source_head_digest: Some(source_two.source_head_digest.clone()),
            dependency_lock_digest: Some(source_two.dependency_lock_digest.clone()),
            build_profile_version: Some(source_two.build_profile_version.clone()),
            build_generation: 2,
            updated_at: 26,
        })
        .await
        .unwrap();
    let operation_two = Uuid::now_v7().to_string();
    let artifact_two_id = Uuid::now_v7().to_string();
    let release_two_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        2,
        &source_two,
        &operation_two,
        27,
    )
    .await;
    let artifact_two = artifact(
        &owner,
        &artifact_two_id,
        &"5".repeat(64),
        &"6".repeat(64),
        28,
    );
    let digest_two = artifact_two.artifact_digest.clone();
    let mut release_two = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_two,
        &release_two_id,
        &digest_two,
        &operation_two,
        &"f".repeat(64),
        &"c".repeat(64),
        29,
    );
    set_release_build_generation(&mut release_two, 2);
    let ready_two = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: operation_two,
            expected_product_revision: authorized.product.product_revision,
            expected_pointer_revision: authorized.product.pointer_revision,
            expected_project_revision: 2,
            expected_build_generation: 2,
            artifact: artifact_two,
            release: release_two,
            bounded_log_tail: vec![],
            finished_at_ms: 30,
        })
        .await
        .unwrap();
    let source_three = managed_source("sources/owner/project/source", '7', 'c', 3);
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_project_revision: 2,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some(source_three.managed_source_path.clone()),
            source_head_digest: Some(source_three.source_head_digest.clone()),
            dependency_lock_digest: Some(source_three.dependency_lock_digest.clone()),
            build_profile_version: Some(source_three.build_profile_version.clone()),
            build_generation: 3,
            updated_at: 31,
        })
        .await
        .unwrap();
    let stale_auto_publish = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: ready_two.product.product_revision,
            expected_pointer_revision: ready_two.product.pointer_revision,
            expected_active_release_epoch: 1,
            expected_ready_release_id: release_two_id.clone(),
            expected_ready_release_digest: digest_two.clone(),
            expected_active_release_digest: Some(digest_one.clone()),
            target_catalog_digest: "2".repeat(64),
            auto_publish_guard: Some(MiniAppM1AutoPublishGuard {
                authorization_id: authorization_id.clone(),
                authorization_revision: 1,
                project_id: PROJECT_ID.to_owned(),
                project_revision: 2,
                source_head_digest: "f".repeat(64),
                dependency_lock_digest: "c".repeat(64),
                build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.to_owned(),
                build_generation: 2,
            }),
            updated_at: 32,
        })
        .await
        .unwrap_err();
    assert!(stale_auto_publish.to_string().contains("Project head advanced"));
    let retained = repository.get(&owner, MINIAPP_ID).await.unwrap().unwrap();
    assert_eq!(
        retained.product.active_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(
        retained.product.ready_release_id.as_deref(),
        Some(release_two_id.as_str())
    );
    assert_eq!(retained.product.active_release_epoch, 1);

    let operation_three = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        3,
        &source_three,
        &operation_three,
        33,
    )
    .await;
    let revoked = repository
        .set_auto_publish_cas(&SetMiniAppM1AutoPublishParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: retained.product.product_revision,
            expected_pointer_revision: retained.product.pointer_revision,
            expected_authorization_revision: Some(1),
            authorization_id,
            enabled: false,
            user_authorized_at_ms: 25,
            updated_at: 34,
        })
        .await
        .unwrap();
    assert!(!revoked.auto_publish_authorization.as_ref().unwrap().enabled);

    let artifact_three_id = Uuid::now_v7().to_string();
    let release_three_id = Uuid::now_v7().to_string();
    let artifact_three = artifact(
        &owner,
        &artifact_three_id,
        &"8".repeat(64),
        &"9".repeat(64),
        35,
    );
    let mut release_three = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_three,
        &release_three_id,
        &"8".repeat(64),
        &operation_three,
        &"7".repeat(64),
        &"c".repeat(64),
        36,
    );
    set_release_build_generation(&mut release_three, 3);
    let completed = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: operation_three.clone(),
            expected_product_revision: retained.product.product_revision,
            expected_pointer_revision: retained.product.pointer_revision,
            expected_project_revision: 3,
            expected_build_generation: 3,
            artifact: artifact_three,
            release: release_three,
            bounded_log_tail: vec![],
            finished_at_ms: 36,
        })
        .await
        .unwrap();
    assert_eq!(
        completed.product.ready_release_id.as_deref(),
        Some(release_three_id.as_str())
    );
    assert_eq!(
        completed.product.active_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert!(!completed.auto_publish_authorization.unwrap().enabled);
    assert_eq!(
        repository
            .get_build_operation(&owner, MINIAPP_ID, &operation_three)
            .await
            .unwrap()
            .unwrap()
            .state,
        "succeeded"
    );
}

#[tokio::test]
async fn build_failure_cancel_and_owner_isolation_keep_release_pointers_unchanged() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let other_owner = insert_other_owner(database.pool()).await;
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);

    let failed_operation_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &failed_operation_id,
        20,
    )
    .await;
    let failed = repository
        .finish_build_operation(&FinishMiniAppM1BuildOperationParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id: failed_operation_id.clone(),
            state: ProductOperationState::Failed,
            progress_percent: 40,
            last_error_code: Some("miniapp_build_failed".to_owned()),
            bounded_log_tail: vec!["build failed".to_owned()],
            finished_at_ms: 21,
        })
        .await
        .unwrap();
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.last_error_code.as_deref(), Some("miniapp_build_failed"));
    let late_finish = repository
        .cancel_build_operation(&CancelMiniAppM1BuildOperationParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id: failed_operation_id,
            bounded_log_tail: vec![],
            finished_at_ms: 22,
        })
        .await
        .unwrap_err();
    assert!(late_finish.to_string().contains("terminal"));

    let canceled_operation_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &canceled_operation_id,
        30,
    )
    .await;
    assert!(repository
        .get_build_operation(&other_owner, MINIAPP_ID, &canceled_operation_id)
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .list_build_operations(&other_owner, MINIAPP_ID)
        .await
        .unwrap()
        .is_empty());
    assert!(repository
        .cancel_build_operation(&CancelMiniAppM1BuildOperationParams {
            owner_user_id: other_owner,
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id: canceled_operation_id.clone(),
            bounded_log_tail: vec!["foreign cancel".to_owned()],
            finished_at_ms: 31,
        })
        .await
        .is_err());
    assert_eq!(
        repository
            .get_build_operation(&owner, MINIAPP_ID, &canceled_operation_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );

    let canceled = repository
        .cancel_build_operation(&CancelMiniAppM1BuildOperationParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id: canceled_operation_id.clone(),
            bounded_log_tail: vec!["build canceled".to_owned()],
            finished_at_ms: 32,
        })
        .await
        .unwrap();
    assert_eq!(canceled.state, "canceled");
    let late_failure = repository
        .finish_build_operation(&FinishMiniAppM1BuildOperationParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id: canceled_operation_id,
            state: ProductOperationState::Failed,
            progress_percent: 0,
            last_error_code: Some("late_failure".to_owned()),
            bounded_log_tail: vec![],
            finished_at_ms: 33,
        })
        .await
        .unwrap_err();
    assert!(late_failure.to_string().contains("terminal"));

    let snapshot = repository.get(&owner, MINIAPP_ID).await.unwrap().unwrap();
    assert_eq!(snapshot.product.product_revision, 1);
    assert_eq!(snapshot.product.pointer_revision, 1);
    assert_eq!(snapshot.library_revision, 1);
    assert!(snapshot.ready_release.is_none());
    assert!(snapshot.active_release.is_none());
    assert!(snapshot.previous_release.is_none());
    let operations = repository
        .list_build_operations(&owner, MINIAPP_ID)
        .await
        .unwrap();
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].state, "canceled");
    assert_eq!(operations[1].state, "failed");
}

#[tokio::test]
async fn concurrent_build_start_is_single_flight_per_miniapp() {
    let directory = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database(&directory.path().join("miniapp-build-race.db"))
        .await
        .unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);
    repository
        .create_with_source(&CreateMiniAppM1WithSourceParams {
            create: create_params(
                &owner,
                MINIAPP_ID,
                PROJECT_ID,
                0,
                MiniAppM1Kind::UiOnly,
                10,
            ),
            source: source.clone(),
        })
        .await
        .unwrap();

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let first_id = Uuid::now_v7().to_string();
    let second_id = Uuid::now_v7().to_string();
    let first = {
        let barrier = Arc::clone(&barrier);
        let repository = repository.clone();
        let owner = owner.clone();
        let source = source.clone();
        let operation_id = first_id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            repository
                .start_build_operation(&StartMiniAppM1BuildOperationParams {
                    owner_user_id: owner,
                    miniapp_id: MINIAPP_ID.to_owned(),
                    project_id: PROJECT_ID.to_owned(),
                    operation_id,
                    expected_project_revision: 1,
                    expected_source: source,
                    bounded_log_tail: vec![],
                    started_at_ms: 20,
                })
                .await
        })
    };
    let second = {
        let barrier = Arc::clone(&barrier);
        let repository = repository.clone();
        let owner = owner.clone();
        let source = source.clone();
        let operation_id = second_id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            repository
                .start_build_operation(&StartMiniAppM1BuildOperationParams {
                    owner_user_id: owner,
                    miniapp_id: MINIAPP_ID.to_owned(),
                    project_id: PROJECT_ID.to_owned(),
                    operation_id,
                    expected_project_revision: 1,
                    expected_source: source,
                    bounded_log_tail: vec![],
                    started_at_ms: 20,
                })
                .await
        })
    };
    barrier.wait().await;
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    let winner = match (first, second) {
        (Ok(operation), Err(error)) | (Err(error), Ok(operation)) => {
            assert!(error.to_string().contains("running Build"));
            operation
        }
        (first, second) => panic!("expected one Build winner, got {first:?} and {second:?}"),
    };
    assert_eq!(winner.owner_kind, "miniapp");
    assert_eq!(winner.kind, "build");
    assert_eq!(winner.state, "running");
    assert_eq!(
        repository
            .list_build_operations(&owner, MINIAPP_ID)
            .await
            .unwrap()
            .len(),
        1
    );
    let canceled = repository
        .cancel_build_operation(&CancelMiniAppM1BuildOperationParams {
            owner_user_id: owner,
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id: winner.operation_id,
            bounded_log_tail: vec!["cleanup".to_owned()],
            finished_at_ms: 21,
        })
        .await
        .unwrap();
    assert_eq!(canceled.state, "canceled");
}

#[tokio::test]
async fn concurrent_lifecycle_and_snapshot_reads_never_observe_catalog_half_state() {
    let directory = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database(&directory.path().join("miniapp-snapshot-race.db"))
        .await
        .unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);
    let operation_id = Uuid::now_v7().to_string();
    let artifact_id = Uuid::now_v7().to_string();
    let release_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &operation_id,
        20,
    )
    .await;
    let artifact = artifact(
        &owner,
        &artifact_id,
        &"d".repeat(64),
        &"e".repeat(64),
        21,
    );
    let digest = artifact.artifact_digest.clone();
    let release = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact,
        &release_id,
        &digest,
        &operation_id,
        &"b".repeat(64),
        &"c".repeat(64),
        22,
    );
    let ready = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id,
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact,
            release,
            bounded_log_tail: vec![],
            finished_at_ms: 23,
        })
        .await
        .unwrap();
    let published = repository
        .publish_ready_cas(&PublishMiniAppM1ReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: ready.product.product_revision,
            expected_pointer_revision: ready.product.pointer_revision,
            expected_active_release_epoch: 0,
            expected_ready_release_id: release_id,
            expected_ready_release_digest: digest.clone(),
            expected_active_release_digest: None,
            target_catalog_digest: "1".repeat(64),
            auto_publish_guard: None,
            updated_at: 24,
        })
        .await
        .unwrap();
    repository
        .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: published.product.product_revision,
            expected_pointer_revision: published.product.pointer_revision,
            expected_active_release_digest: Some(digest.clone()),
            enabled: true,
            updated_at: 25,
        })
        .await
        .unwrap();

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let writer = {
        let repository = repository.clone();
        let owner = owner.clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            for _ in 0..40 {
                let snapshot = repository
                    .get(&owner, MINIAPP_ID)
                    .await?
                    .expect("MiniApp must remain present");
                let enable = snapshot.product.lifecycle == "disabled";
                repository
                    .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
                        owner_user_id: owner.clone(),
                        miniapp_id: MINIAPP_ID.to_owned(),
                        expected_product_revision: snapshot.product.product_revision,
                        expected_pointer_revision: snapshot.product.pointer_revision,
                        expected_active_release_digest: snapshot
                            .product
                            .active_release_digest
                            .clone(),
                        enabled: enable,
                        updated_at: snapshot.product.updated_at.saturating_add(1),
                    })
                    .await?;
            }
            Ok::<(), nomifun_db::DbError>(())
        })
    };
    let reader = {
        let repository = repository.clone();
        let owner = owner.clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            for _ in 0..200 {
                let snapshot = repository
                    .get(&owner, MINIAPP_ID)
                    .await?
                    .expect("MiniApp must remain present");
                match snapshot.product.lifecycle.as_str() {
                    "enabled" => {
                        let catalog = snapshot
                            .catalog_publication
                            .as_ref()
                            .expect("enabled snapshot requires Catalog");
                        assert_eq!(
                            snapshot.product.active_release_id.as_deref(),
                            Some(catalog.active_release_id.as_str())
                        );
                        assert_eq!(
                            snapshot.product.active_release_epoch,
                            catalog.active_release_epoch
                        );
                    }
                    "disabled" => assert!(snapshot.catalog_publication.is_none()),
                    lifecycle => panic!("unexpected lifecycle {lifecycle}"),
                }
                tokio::task::yield_now().await;
            }
            Ok::<(), nomifun_db::DbError>(())
        })
    };
    barrier.wait().await;
    writer.await.unwrap().unwrap();
    reader.await.unwrap().unwrap();
}

#[tokio::test]
async fn atomic_build_ready_timestamp_cas_rolls_back_all_writes() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);
    let operation_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &operation_id,
        20,
    )
    .await;

    sqlx::query(
        "UPDATE miniapp_library_state
         SET updated_at = 1000
         WHERE owner_user_id = ?",
    )
    .bind(&owner)
    .execute(database.pool())
    .await
    .unwrap();
    let artifact = artifact(
        &owner,
        &Uuid::now_v7().to_string(),
        &"d".repeat(64),
        &"e".repeat(64),
        21,
    );
    let release = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact,
        &Uuid::now_v7().to_string(),
        &"d".repeat(64),
        &operation_id,
        &source.source_head_digest,
        &source.dependency_lock_digest,
        22,
    );
    let error = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: operation_id.clone(),
            expected_product_revision: 1,
            expected_pointer_revision: 1,
            expected_project_revision: 1,
            expected_build_generation: 1,
            artifact,
            release,
            bounded_log_tail: vec!["ready commit".to_owned()],
            finished_at_ms: 30,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("library state"));

    let operation = repository
        .get_build_operation(&owner, MINIAPP_ID, &operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.state, "running");
    assert!(operation.finished_at_ms.is_none());
    let snapshot = repository.get(&owner, MINIAPP_ID).await.unwrap().unwrap();
    assert_eq!(snapshot.product.product_revision, 1);
    assert_eq!(snapshot.product.pointer_revision, 1);
    assert!(snapshot.ready_release.is_none());
    assert_eq!(snapshot.library_revision, 1);
    let artifact_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_release_artifacts")
            .fetch_one(database.pool())
            .await
            .unwrap();
    let release_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_releases")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(artifact_count, 0);
    assert_eq!(release_count, 0);
}

#[tokio::test]
async fn product_config_and_credential_references_use_exact_owner_scoped_cas() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let other_owner = insert_other_owner(database.pool()).await;
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    repository
        .create(&CreateMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_library_revision: 0,
            display_name: "Runtime state app".to_owned(),
            description: None,
            icon_asset_id: None,
            kind: MiniAppM1Kind::UiOnly,
            materialized_catalog_digest: "a".repeat(64),
            config_schema_json: r#"{"type":"object"}"#.to_owned(),
            config_json: "{}".to_owned(),
            created_at: 10,
        })
        .await
        .unwrap();

    let configured = repository
        .update_config_cas(
            &owner,
            MINIAPP_ID,
            1,
            1,
            1,
            r#"{"type":"object"}"#,
            r#"{"theme":"dark"}"#,
            20,
        )
        .await
        .unwrap();
    assert_eq!(configured.product.product_revision, 2);
    assert_eq!(configured.product.config_revision, 2);
    assert_eq!(configured.product.config_json, r#"{"theme":"dark"}"#);
    assert_eq!(configured.library_revision, 2);

    let stale_schema = repository
        .update_config_cas(
            &owner,
            MINIAPP_ID,
            2,
            1,
            2,
            r#"{"type":"object","properties":{}}"#,
            r#"{"theme":"light"}"#,
            21,
        )
        .await
        .unwrap_err();
    assert!(stale_schema.to_string().contains("exact CAS"));

    let bindings = BTreeMap::from([
        ("primary".to_owned(), "credential-primary".to_owned()),
        ("secondary".to_owned(), "credential-secondary".to_owned()),
    ]);
    let bound = repository
        .replace_credential_bindings_cas(
            &owner,
            MINIAPP_ID,
            2,
            1,
            1,
            &bindings,
            30,
        )
        .await
        .unwrap();
    assert_eq!(bound.product.product_revision, 3);
    assert_eq!(bound.product.credential_bindings_revision, 2);
    assert_eq!(bound.library_revision, 3);
    assert_eq!(bound.credential_bindings.len(), 2);
    assert_eq!(bound.credential_bindings[0].slot_key, "primary");
    assert_eq!(
        bound.credential_bindings[0].credential_id,
        "credential-primary"
    );

    let stale = repository
        .replace_credential_bindings_cas(
            &owner,
            MINIAPP_ID,
            2,
            1,
            1,
            &BTreeMap::new(),
            31,
        )
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("exact CAS"));

    let cross_owner = repository
        .update_config_cas(
            &other_owner,
            MINIAPP_ID,
            3,
            1,
            2,
            r#"{"type":"object"}"#,
            r#"{"theme":"light"}"#,
            32,
        )
        .await
        .unwrap_err();
    assert!(cross_owner.to_string().contains("not found"));
}

#[tokio::test]
async fn config_and_credential_mutations_are_blocked_during_a_running_build() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    create_editable_app(&repository, &owner).await;
    let source = managed_source("sources/owner/project/source", 'b', 'c', 1);
    let operation_id = Uuid::now_v7().to_string();
    start_build(
        &repository,
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        1,
        &source,
        &operation_id,
        20,
    )
    .await;

    let snapshot = repository.get(&owner, MINIAPP_ID).await.unwrap().unwrap();
    let config_error = repository
        .update_config_cas(
            &owner,
            MINIAPP_ID,
            snapshot.product.product_revision,
            snapshot.product.pointer_revision,
            snapshot.product.config_revision,
            &snapshot.product.config_schema_json,
            &snapshot.product.config_json,
            21,
        )
        .await
        .unwrap_err();
    assert!(config_error.to_string().contains("running Build"));

    let credential_error = repository
        .replace_credential_bindings_cas(
            &owner,
            MINIAPP_ID,
            snapshot.product.product_revision,
            snapshot.product.pointer_revision,
            snapshot.product.credential_bindings_revision,
            &BTreeMap::new(),
            21,
        )
        .await
        .unwrap_err();
    assert!(credential_error.to_string().contains("running Build"));

    let canceled = repository
        .cancel_build_operation(&CancelMiniAppM1BuildOperationParams {
            owner_user_id: owner,
            miniapp_id: MINIAPP_ID.to_owned(),
            operation_id,
            bounded_log_tail: vec!["cleanup".to_owned()],
            finished_at_ms: 22,
        })
        .await
        .unwrap();
    assert_eq!(canceled.state, "canceled");
}

#[tokio::test]
async fn host_kv_is_owner_and_namespace_scoped_with_checked_revision_cas() {
    let database = init_miniapp_test_database().await;
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let other_owner = insert_other_owner(database.pool()).await;
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    repository
        .create(&CreateMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            expected_library_revision: 0,
            display_name: "KV app".to_owned(),
            description: None,
            icon_asset_id: None,
            kind: MiniAppM1Kind::UiOnly,
            materialized_catalog_digest: "a".repeat(64),
            config_schema_json: r#"{"type":"object"}"#.to_owned(),
            config_json: "{}".to_owned(),
            created_at: 10,
        })
        .await
        .unwrap();

    let first = repository
        .put_kv_cas(
            &owner,
            MINIAPP_ID,
            "surface",
            "state",
            &json!({"value": 1}),
            None,
            11,
        )
        .await
        .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(first.value_json, r#"{"value":1}"#);

    let isolated = repository
        .put_kv_cas(
            &owner,
            MINIAPP_ID,
            "preview",
            "state",
            &json!({"value": "preview"}),
            None,
            12,
        )
        .await
        .unwrap();
    assert_eq!(isolated.revision, 1);

    let updated = repository
        .put_kv_cas(
            &owner,
            MINIAPP_ID,
            "surface",
            "state",
            &json!({"value": 2}),
            Some(1),
            13,
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);
    let fetched = repository
        .get_kv(&owner, MINIAPP_ID, "surface", "state")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.value_json, r#"{"value":2}"#);

    let stale = repository
        .put_kv_cas(
            &owner,
            MINIAPP_ID,
            "surface",
            "state",
            &json!({"value": 3}),
            Some(1),
            14,
        )
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("CAS"));

    let cross_owner = repository
        .get_kv(&other_owner, MINIAPP_ID, "surface", "state")
        .await
        .unwrap_err();
    assert!(cross_owner.to_string().contains("not found"));

    let stale_delete = repository
        .delete_kv_cas(&owner, MINIAPP_ID, "surface", "state", 1, 15)
        .await
        .unwrap_err();
    assert!(stale_delete.to_string().contains("CAS"));
    assert!(
        repository
            .delete_kv_cas(&owner, MINIAPP_ID, "surface", "state", 2, 15)
            .await
            .unwrap()
    );
    let stale_tombstone_delete = repository
        .delete_kv_cas(&owner, MINIAPP_ID, "surface", "state", 2, 16)
        .await
        .unwrap_err();
    assert!(stale_tombstone_delete.to_string().contains("CAS"));
    assert!(
        !repository
            .delete_kv_cas(&owner, MINIAPP_ID, "surface", "state", 3, 17)
            .await
            .unwrap()
    );
    assert!(
        repository
            .get_kv(&owner, MINIAPP_ID, "surface", "state")
            .await
            .unwrap()
            .is_none()
    );
    let recreated = repository
        .put_kv_cas(
            &owner,
            MINIAPP_ID,
            "surface",
            "state",
            &json!({"value": 4}),
            Some(3),
            18,
        )
        .await
        .unwrap();
    assert_eq!(recreated.revision, 4);
    assert_eq!(recreated.key_generation, 2);
    assert!(!recreated.is_tombstone);
    assert_eq!(recreated.value_json, r#"{"value":4}"#);
    let stale_recreate = repository
        .put_kv_cas(
            &owner,
            MINIAPP_ID,
            "surface",
            "state",
            &json!({"value": 5}),
            Some(3),
            19,
        )
        .await
        .unwrap_err();
    assert!(stale_recreate.to_string().contains("CAS"));
    assert!(
        repository
            .get_kv(&owner, MINIAPP_ID, "preview", "state")
            .await
            .unwrap()
            .is_some()
    );
}
