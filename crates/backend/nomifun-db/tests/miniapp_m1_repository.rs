use std::collections::BTreeMap;
use std::sync::Arc;

use nomifun_agent_contracts::MINIAPP_RELEASE_PROFILE_VERSION;
use nomifun_db::{
    CancelMiniAppM1BuildOperationParams, CommitMiniAppM1PointerStateParams,
    CreateMiniAppM1Params, CreateMiniAppM1WithSourceParams,
    FinishMiniAppM1BuildAndRecordReadyParams, FinishMiniAppM1BuildOperationParams,
    IMiniAppM1Repository, MiniAppM1Kind, MiniAppM1ManagedSourceLineage,
    MiniAppM1ProjectSourceState, MiniAppM1Snapshot, MiniAppReleaseArtifactRow,
    MiniAppReleaseRow, ProductOperationState, SqliteMiniAppM1Repository,
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
    artifact_digest: &str,
    manifest_digest: &str,
    created_at: i64,
) -> MiniAppReleaseArtifactRow {
    MiniAppReleaseArtifactRow {
        id: 0,
        artifact_id: artifact_id.to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_digest: artifact_digest.to_owned(),
        manifest_digest: manifest_digest.to_owned(),
        artifact_record_json: json!({
            "artifact_id": artifact_id,
            "artifact_digest": artifact_digest,
            "manifest_digest": manifest_digest
        })
        .to_string(),
        managed_path: format!("artifacts/{artifact_digest}"),
        created_at,
    }
}

fn release(
    owner: &str,
    miniapp_id: &str,
    project_id: &str,
    artifact: &MiniAppReleaseArtifactRow,
    release_id: &str,
    release_digest: &str,
    operation_id: &str,
    source_digest: &str,
    lock_digest: &str,
    created_at: i64,
) -> MiniAppReleaseRow {
    MiniAppReleaseRow {
        id: 0,
        release_id: release_id.to_owned(),
        miniapp_id: miniapp_id.to_owned(),
        owner_user_id: owner.to_owned(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_digest: artifact.artifact_digest.clone(),
        manifest_digest: artifact.manifest_digest.clone(),
        release_digest: release_digest.to_owned(),
        origin_kind: "build".to_owned(),
        origin_operation_id: operation_id.to_owned(),
        source_kind: "managed".to_owned(),
        project_id: Some(project_id.to_owned()),
        source_snapshot_digest: Some(source_digest.to_owned()),
        dependency_lock_digest: Some(lock_digest.to_owned()),
        build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.to_owned()),
        build_generation: Some(1),
        release_record_json: json!({
            "release_id": release_id,
            "release_digest": release_digest
        })
        .to_string(),
        created_at,
    }
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
    let release_one = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_one,
        &release_one_id,
        &"d".repeat(64),
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
    let release_two = release(
        &owner,
        MINIAPP_ID,
        PROJECT_ID,
        &artifact_two,
        &release_two_id,
        &"1".repeat(64),
        &op_two,
        &"b".repeat(64),
        &"c".repeat(64),
        42,
    );
    let ready_two = repository
        .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            project_id: PROJECT_ID.to_owned(),
            operation_id: op_two,
            expected_product_revision: 2,
            expected_pointer_revision: 2,
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
        .commit_pointer_state_cas(&CommitMiniAppM1PointerStateParams {
            owner_user_id: owner.clone(),
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 3,
            expected_pointer_revision: 3,
            expected_active_release_epoch: 0,
            ready_release_id: Some(release_two_id.clone()),
            ready_release_digest: Some("1".repeat(64)),
            active_release_id: Some(release_one_id.clone()),
            active_release_digest: Some("d".repeat(64)),
            previous_release_id: None,
            previous_release_digest: None,
            active_release_epoch: 1,
            materialized_catalog_digest: "4".repeat(64),
            updated_at: 44,
        })
        .await
        .unwrap();
    assert_eq!(
        activated.product.active_release_id.as_deref(),
        Some(release_one_id.as_str())
    );
    assert_eq!(activated.product.active_release_epoch, 1);

    let stale = repository
        .commit_pointer_state_cas(&CommitMiniAppM1PointerStateParams {
            owner_user_id: owner,
            miniapp_id: MINIAPP_ID.to_owned(),
            expected_product_revision: 3,
            expected_pointer_revision: 3,
            expected_active_release_epoch: 0,
            ready_release_id: ready_two.product.ready_release_id,
            ready_release_digest: ready_two.product.ready_release_digest,
            active_release_id: None,
            active_release_digest: None,
            previous_release_id: None,
            previous_release_digest: None,
            active_release_epoch: 0,
            materialized_catalog_digest: "5".repeat(64),
            updated_at: 45,
        })
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("CAS"));
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
    assert!(
        !repository
            .delete_kv_cas(&owner, MINIAPP_ID, "surface", "state", 2, 16)
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
    assert!(
        repository
            .get_kv(&owner, MINIAPP_ID, "preview", "state")
            .await
            .unwrap()
            .is_some()
    );
}
