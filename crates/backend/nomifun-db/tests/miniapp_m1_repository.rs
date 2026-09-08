use std::collections::BTreeMap;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ArtifactId, DigestHex, JavaScriptBuildProfile, LocalizedMetadata,
    MiniAppReleaseArtifactV1, MiniAppReleaseFile, MiniAppReleaseRef,
    MiniAppReleaseV1Manifest, MiniAppReadyOrigin, MiniAppReadyRelease,
    MiniAppResourceContract, MiniAppSourceLineage, MiniAppUiReleaseDescriptor,
    PackageId, PackageRef, StrictJsonValue, VersionString,
    MINIAPP_BRIDGE_CONTRACT_VERSION, MINIAPP_M1_SCHEMA_VERSION,
    MINIAPP_RELEASE_PROFILE_VERSION, canonical_ui_tree_digest, digest_bytes,
    digest_payload,
};
use nomifun_db::{
    CancelMiniAppM1BuildOperationParams, CloseMiniAppM1SurfaceSessionParams,
    CommitMiniAppM1LifecycleParams,
    CreateMiniAppM1Params, CreateMiniAppM1WithSourceParams,
    ExecuteMiniAppM1SurfaceKvParams,
    FinishMiniAppM1BuildAndRecordReadyParams, FinishMiniAppM1BuildOperationParams,
    IMiniAppM1Repository, MiniAppM1AutoPublishGuard, MiniAppM1Kind,
    MiniAppM1ManagedSourceLineage, MiniAppM1ProjectSourceState,
    MiniAppM1Snapshot, MiniAppM1SurfaceKvOperation, MiniAppM1SurfaceKvResult,
    MiniAppReleaseArtifactRow, MiniAppReleaseRow,
    OpenMiniAppM1SurfaceSessionParams, ProductOperationState,
    PublishMiniAppM1ReadyParams, ResolveMiniAppM1SurfaceSessionParams,
    RollbackMiniAppM1PreviousParams, SetMiniAppM1AutoPublishParams,
    SqliteMiniAppM1Repository,
    StartMiniAppM1BuildOperationParams,
    UpdateMiniAppM1ProjectSourceParams, installation_owner_id,
};
use serde_json::json;
use sqlx::migrate::{Migrate, Migrator};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const MINIAPP_ID: &str = "0190f5fe-7c00-7000-8000-000000000101";
const PROJECT_ID: &str = "0190f5fe-7c00-7000-8000-000000000102";

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
    let html = format!(
        "<!doctype html><html><body><main><h1>fixture-{artifact_seed}</h1></main></body></html>"
    )
    .into_bytes();
    let file = MiniAppReleaseFile {
        normalized_relative_path: "ui/index.html".to_owned(),
        digest: digest_bytes(&html),
        size_bytes: html.len() as u64,
    };
    let files = vec![file];
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
        ui: MiniAppUiReleaseDescriptor {
            entrypoint: "ui/index.html".to_owned(),
            entrypoint_digest: files[0].digest.clone(),
            ui_tree_digest: canonical_ui_tree_digest(&files).unwrap(),
        },
        service: None,
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
        migrations: Vec::new(),
    };
    MiniAppReleaseArtifactV1::new(
        ArtifactId::from(artifact_id),
        manifest,
        files,
    )
    .unwrap()
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
