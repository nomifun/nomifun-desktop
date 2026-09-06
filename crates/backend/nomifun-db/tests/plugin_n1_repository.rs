use std::path::Path;

use nomifun_db::{
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams, DbError,
    FinishProductOperationParams, IPluginN1Repository, PluginCandidateOrigin,
    ProductOperationKind, ProductOperationState, PutPluginKvParams,
    RecordPluginCandidateTestReceiptParams, RecordPluginReadyCandidateParams,
    RestorePluginMountParams, SqlitePluginN1Repository, StartProductOperationParams,
    UninstallPluginMountParams, UpdatePluginProjectSourceParams, init_database,
    init_database_memory, installation_owner_id, MAX_PRODUCT_OPERATION_LOG_LINE_CHARS,
    MAX_PRODUCT_OPERATION_LOG_LINES,
};
use serde_json::json;
use sqlx::migrate::{Migrate, Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

fn id() -> String {
    nomifun_common::generate_id()
}

fn digest(byte: char) -> String {
    byte.to_string().repeat(64)
}

async fn migrate_through(pool: &sqlx::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= maximum_version)
    {
        connection.apply(migration).await.unwrap();
    }
}

async fn succeed_operation(
    repo: &SqlitePluginN1Repository,
    kind: ProductOperationKind,
    owner_kind: &str,
    owner_id: &str,
    timestamp: i64,
) -> String {
    let operation_id = id();
    repo.start_operation(&StartProductOperationParams {
        operation_id: operation_id.clone(),
        kind,
        owner_kind: owner_kind.into(),
        owner_id: owner_id.into(),
        progress_percent: (kind != ProductOperationKind::MiniappPermanentDelete).then_some(0),
        bounded_log_tail: vec!["started".into()],
        started_at_ms: timestamp,
    })
    .await
    .unwrap();
    repo.finish_operation(&FinishProductOperationParams {
        operation_id: operation_id.clone(),
        state: ProductOperationState::Succeeded,
        progress_percent: (kind != ProductOperationKind::MiniappPermanentDelete).then_some(100),
        last_error_code: None,
        bounded_log_tail: vec!["started".into(), "succeeded".into()],
        finished_at_ms: timestamp + 1,
    })
    .await
    .unwrap();
    operation_id
}

struct ManagedFixture {
    pool: sqlx::SqlitePool,
    repo: SqlitePluginN1Repository,
    project_id: String,
    source_digest: String,
    lock_digest: String,
    artifact_id: String,
    artifact_digest: String,
    candidate_id: String,
}

async fn managed_fixture() -> ManagedFixture {
    let database = init_database_memory().await.unwrap();
    let pool = database.pool().clone();
    let owner_user_id = installation_owner_id(&pool).await.unwrap();
    let repo = SqlitePluginN1Repository::new(pool.clone());
    let project_id = id();
    repo.create_project(&CreatePluginProjectParams {
        project_id: project_id.clone(),
        owner_user_id,
        package_id: "dev.nomifun.fixture".into(),
        managed_source_path: Some("plugin-projects/fixture".into()),
        source_head_digest: None,
        dependency_lock_digest: None,
        created_at: 1,
    })
    .await
    .unwrap();
    let source_digest = digest('1');
    let lock_digest = digest('2');
    repo.update_project_source_cas(&UpdatePluginProjectSourceParams {
        project_id: project_id.clone(),
        expected_generation: 0,
        source_head_digest: source_digest.clone(),
        dependency_lock_digest: Some(lock_digest.clone()),
        updated_at: 2,
    })
    .await
    .unwrap();

    let artifact_id = id();
    let artifact_digest = digest('a');
    repo.put_artifact(&CreatePluginArtifactParams {
        artifact_id: artifact_id.clone(),
        artifact_digest: artifact_digest.clone(),
        package_id: "dev.nomifun.fixture".into(),
        package_version: "1.0.0".into(),
        manifest_digest: digest('3'),
        manifest: json!({"schemaVersion": "plugin-package-v1", "packageVersion": "1.0.0"}),
        managed_path: format!("plugin-artifacts/{artifact_id}"),
        created_at: 2,
    })
    .await
    .unwrap();
    let operation_id = succeed_operation(
        &repo,
        ProductOperationKind::Build,
        "plugin_project",
        &project_id,
        3,
    )
    .await;
    let candidate_id = id();
    let candidate = repo
        .record_ready_candidate(&RecordPluginReadyCandidateParams {
            candidate_id: candidate_id.clone(),
            project_id: project_id.clone(),
            candidate_digest: digest('4'),
            origin: PluginCandidateOrigin::Build,
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            base_target_digest: None,
            source_snapshot_digest: Some(source_digest.clone()),
            dependency_lock_digest: Some(lock_digest.clone()),
            contract_diff: json!({"compatibility": "initial"}),
            origin_operation_id: operation_id,
            expected_generation: 1,
            created_at: 5,
        })
        .await
        .unwrap();
    assert_eq!(candidate.target_package_id, "dev.nomifun.fixture");
    assert_eq!(candidate.target_package_version, "1.0.0");
    assert_eq!(candidate.target_manifest_digest, digest('3'));

    ManagedFixture {
        pool,
        repo,
        project_id,
        source_digest,
        lock_digest,
        artifact_id,
        artifact_digest,
        candidate_id,
    }
}

async fn add_managed_candidate(
    fixture: &ManagedFixture,
    generation: i64,
    source_digest: &str,
    lock_digest: &str,
    artifact_digit: char,
    package_version: &str,
    base_target_digest: Option<String>,
    timestamp: i64,
) -> (String, String, String) {
    let artifact_id = id();
    let artifact_digest = digest(artifact_digit);
    fixture
        .repo
        .put_artifact(&CreatePluginArtifactParams {
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            package_id: "dev.nomifun.fixture".into(),
            package_version: package_version.into(),
            manifest_digest: digest(
                char::from_digit(artifact_digit.to_digit(16).unwrap() ^ 1, 16).unwrap(),
            ),
            manifest: json!({
                "schemaVersion": "plugin-package-v1",
                "packageVersion": package_version
            }),
            managed_path: format!("plugin-artifacts/{artifact_id}"),
            created_at: timestamp,
        })
        .await
        .unwrap();
    let operation_id = succeed_operation(
        &fixture.repo,
        ProductOperationKind::Build,
        "plugin_project",
        &fixture.project_id,
        timestamp,
    )
    .await;
    let candidate_id = id();
    fixture
        .repo
        .record_ready_candidate(&RecordPluginReadyCandidateParams {
            candidate_id: candidate_id.clone(),
            project_id: fixture.project_id.clone(),
            candidate_digest: digest(
                char::from_digit(artifact_digit.to_digit(16).unwrap() ^ 2, 16).unwrap(),
            ),
            origin: PluginCandidateOrigin::Build,
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            base_target_digest,
            source_snapshot_digest: Some(source_digest.into()),
            dependency_lock_digest: Some(lock_digest.into()),
            contract_diff: json!({"compatibility": "compatible"}),
            origin_operation_id: operation_id,
            expected_generation: generation,
            created_at: timestamp + 2,
        })
        .await
        .unwrap();
    (candidate_id, artifact_id, artifact_digest)
}

#[tokio::test]
async fn migrations_are_clean_start_preserve_legacy_miniapps_and_restart_through_068() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plugin-n1.db");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    migrate_through(&pool, 66).await;
    let owner_id = id();
    sqlx::query(
        "INSERT INTO users (user_id, username, password_hash, jwt_secret, created_at, updated_at)
         VALUES (?, ?, '', '', 0, 0)",
    )
    .bind(&owner_id)
    .bind(&owner_id)
    .execute(&pool)
    .await
    .unwrap();
    let miniapp_id = id();
    sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size, created_at, updated_at
         ) VALUES (?, ?, 'legacy', '', '<p/>', 4, 1, 1)",
    )
    .bind(&miniapp_id)
    .bind(&owner_id)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let database = init_database(Path::new(&path)).await.unwrap();
    let plugin_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table' AND name IN (
            'plugin_artifacts', 'plugin_projects', 'plugin_ready_candidates',
            'plugin_candidate_test_receipts', 'plugin_mounts', 'plugin_mount_revisions',
            'plugin_credential_bindings', 'plugin_kv', 'product_operations'
         )",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(plugin_tables, 9);
    let runtime_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table' AND name = 'javascript_runtime_selection'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(runtime_tables, 1);
    let legacy_html: String =
        sqlx::query_scalar("SELECT html FROM miniapps WHERE miniapp_id = ?")
            .bind(&miniapp_id)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(legacy_html, "<p/>");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM plugin_projects")
            .fetch_one(database.pool())
            .await
            .unwrap(),
        0
    );
    database.close().await;

    let restarted = init_database(Path::new(&path)).await.unwrap();
    let version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(restarted.pool())
        .await
        .unwrap();
    assert_eq!(version, 68);
}

#[tokio::test]
async fn runtime_only_read_only_project_imports_generation_zero_candidate_with_exact_target() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runtime-only-project.db");
    let database = init_database(&path).await.unwrap();
    let owner_user_id = installation_owner_id(database.pool()).await.unwrap();
    let repo = SqlitePluginN1Repository::new(database.pool().clone());
    let project_id = id();
    repo.create_project(&CreatePluginProjectParams {
        project_id: project_id.clone(),
        owner_user_id,
        package_id: "dev.nomifun.runtime-only".into(),
        managed_source_path: None,
        source_head_digest: None,
        dependency_lock_digest: None,
        created_at: 1,
    })
    .await
    .unwrap();
    let artifact_id = id();
    let artifact_digest = digest('a');
    repo.put_artifact(&CreatePluginArtifactParams {
        artifact_id: artifact_id.clone(),
        artifact_digest: artifact_digest.clone(),
        package_id: "dev.nomifun.runtime-only".into(),
        package_version: "2.3.4".into(),
        manifest_digest: digest('b'),
        manifest: json!({"schemaVersion": "plugin-package-v1", "packageVersion": "2.3.4"}),
        managed_path: format!("plugin-artifacts/{artifact_id}"),
        created_at: 1,
    })
    .await
    .unwrap();
    let operation_id = succeed_operation(
        &repo,
        ProductOperationKind::Import,
        "plugin_project",
        &project_id,
        2,
    )
    .await;
    let candidate = repo
        .record_ready_candidate(&RecordPluginReadyCandidateParams {
            candidate_id: id(),
            project_id: project_id.clone(),
            candidate_digest: digest('c'),
            origin: PluginCandidateOrigin::Import,
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            base_target_digest: None,
            source_snapshot_digest: None,
            dependency_lock_digest: None,
            contract_diff: json!({"compatibility": "unknown"}),
            origin_operation_id: operation_id,
            expected_generation: 0,
            created_at: 4,
        })
        .await
        .unwrap();
    assert_eq!(candidate.build_generation, 0);
    assert!(repo
        .update_project_source_cas(&UpdatePluginProjectSourceParams {
            project_id,
            expected_generation: 0,
            source_head_digest: digest('d'),
            dependency_lock_digest: Some(digest('e')),
            updated_at: 5,
        })
        .await
        .is_err());
    assert!(repo
        .start_operation(&StartProductOperationParams {
            operation_id: id(),
            kind: ProductOperationKind::Build,
            owner_kind: "plugin_project".into(),
            owner_id: candidate.project_id.clone(),
            progress_percent: Some(0),
            bounded_log_tail: vec![],
            started_at_ms: 5,
        })
        .await
        .is_err());
    database.close().await;

    let reopened = init_database(&path).await.unwrap();
    let recovered = SqlitePluginN1Repository::new(reopened.pool().clone())
        .get_ready_candidate(&candidate.project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.origin_kind, "import");
    assert_eq!(recovered.artifact_id, artifact_id);
    assert_eq!(recovered.artifact_digest, artifact_digest);
    assert_eq!(recovered.target_package_id, "dev.nomifun.runtime-only");
    assert_eq!(recovered.target_package_version, "2.3.4");
    assert_eq!(recovered.target_manifest_digest, digest('b'));
    assert!(recovered.source_snapshot_digest.is_none());
    assert!(recovered.dependency_lock_digest.is_none());
}

#[tokio::test]
async fn local_version_is_an_author_label_not_a_permanent_digest_claim() {
    let database = init_database_memory().await.unwrap();
    let repo = SqlitePluginN1Repository::new(database.pool().clone());
    let package_id = "dev.nomifun.same-version";
    let package_version = "1.0.0";
    for (artifact_id, artifact_digest, manifest_digest, suffix) in [
        (id(), digest('a'), digest('b'), "first"),
        (id(), digest('c'), digest('d'), "second"),
    ] {
        repo.put_artifact(&CreatePluginArtifactParams {
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            package_id: package_id.into(),
            package_version: package_version.into(),
            manifest_digest,
            manifest: json!({
                "schemaVersion": "plugin-package-v1",
                "packageVersion": package_version,
                "variant": suffix
            }),
            managed_path: format!("plugin-artifacts/{artifact_id}"),
            created_at: 1,
        })
        .await
        .unwrap();
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plugin_artifacts
         WHERE package_id = ? AND package_version = ?",
    )
    .bind(package_id)
    .bind(package_version)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn managed_build_requires_real_source_lock_and_positive_generation() {
    let database = init_database_memory().await.unwrap();
    let owner_user_id = installation_owner_id(database.pool()).await.unwrap();
    let repo = SqlitePluginN1Repository::new(database.pool().clone());
    let project_id = id();
    repo.create_project(&CreatePluginProjectParams {
        project_id: project_id.clone(),
        owner_user_id,
        package_id: "dev.nomifun.managed".into(),
        managed_source_path: Some("plugin-projects/managed".into()),
        source_head_digest: Some(digest('1')),
        dependency_lock_digest: Some(digest('2')),
        created_at: 1,
    })
    .await
    .unwrap();
    let artifact_id = id();
    let artifact_digest = digest('a');
    repo.put_artifact(&CreatePluginArtifactParams {
        artifact_id: artifact_id.clone(),
        artifact_digest: artifact_digest.clone(),
        package_id: "dev.nomifun.managed".into(),
        package_version: "1.0.0".into(),
        manifest_digest: digest('b'),
        manifest: json!({"schemaVersion": "plugin-package-v1"}),
        managed_path: format!("plugin-artifacts/{artifact_id}"),
        created_at: 1,
    })
    .await
    .unwrap();
    let invalid_generation = repo
        .start_operation(&StartProductOperationParams {
            operation_id: id(),
            kind: ProductOperationKind::Build,
            owner_kind: "plugin_project".into(),
            owner_id: project_id.clone(),
            progress_percent: Some(0),
            bounded_log_tail: vec![],
            started_at_ms: 2,
        })
        .await
        .unwrap_err();
    assert!(matches!(invalid_generation, DbError::Conflict(message) if message.contains("positive generation")));

    repo.update_project_source_cas(&UpdatePluginProjectSourceParams {
        project_id: project_id.clone(),
        expected_generation: 0,
        source_head_digest: digest('3'),
        dependency_lock_digest: Some(digest('4')),
        updated_at: 5,
    })
    .await
    .unwrap();
    let valid_build = succeed_operation(
        &repo,
        ProductOperationKind::Build,
        "plugin_project",
        &project_id,
        6,
    )
    .await;
    let missing_lock = repo
        .record_ready_candidate(&RecordPluginReadyCandidateParams {
            candidate_id: id(),
            project_id,
            candidate_digest: digest('d'),
            origin: PluginCandidateOrigin::Build,
            artifact_id,
            artifact_digest,
            base_target_digest: None,
            source_snapshot_digest: Some(digest('3')),
            dependency_lock_digest: None,
            contract_diff: json!({}),
            origin_operation_id: valid_build,
            expected_generation: 1,
            created_at: 8,
        })
        .await
        .unwrap_err();
    assert!(matches!(missing_lock, DbError::Conflict(message) if message.contains("source/dependency")));
}

#[tokio::test]
async fn ready_candidate_generation_cas_preserves_previous_ready_on_stale_build() {
    let fixture = managed_fixture().await;
    let project = fixture
        .repo
        .update_project_source_cas(&UpdatePluginProjectSourceParams {
            project_id: fixture.project_id.clone(),
            expected_generation: 1,
            source_head_digest: digest('5'),
            dependency_lock_digest: Some(digest('6')),
            updated_at: 10,
        })
        .await
        .unwrap();
    assert_eq!(project.build_generation, 2);
    assert_eq!(
        project.ready_candidate_id.as_deref(),
        Some(fixture.candidate_id.as_str())
    );
    let operation_id = succeed_operation(
        &fixture.repo,
        ProductOperationKind::Build,
        "plugin_project",
        &fixture.project_id,
        11,
    )
    .await;
    let error = fixture
        .repo
        .record_ready_candidate(&RecordPluginReadyCandidateParams {
            candidate_id: id(),
            project_id: fixture.project_id.clone(),
            candidate_digest: digest('7'),
            origin: PluginCandidateOrigin::Build,
            artifact_id: fixture.artifact_id,
            artifact_digest: fixture.artifact_digest,
            base_target_digest: None,
            source_snapshot_digest: Some(fixture.source_digest),
            dependency_lock_digest: Some(fixture.lock_digest),
            contract_diff: json!({}),
            origin_operation_id: operation_id,
            expected_generation: 1,
            created_at: 13,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(message) if message.contains("generation")));
    let unchanged = fixture
        .repo
        .get_project(&fixture.project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.ready_candidate_id.as_deref(),
        Some(fixture.candidate_id.as_str())
    );
}

#[tokio::test]
async fn replace_rotates_current_previous_rejects_stale_base_and_restore_uses_exact_cas() {
    let fixture = managed_fixture().await;
    let mount_id = id();
    let first = fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id.clone(),
            candidate_id: fixture.candidate_id.clone(),
            expected_project_generation: 1,
            expected_mount_revision: Some(0),
            expected_current_artifact_digest: None,
            new_mount_id: Some(mount_id.clone()),
            new_data_dir_path: Some(format!("plugin-data/{mount_id}")),
            applied_at: 6,
        })
        .await
        .unwrap();
    assert_eq!(first.revision, 1);

    let source = digest('5');
    let lock = digest('6');
    fixture
        .repo
        .update_project_source_cas(&UpdatePluginProjectSourceParams {
            project_id: fixture.project_id.clone(),
            expected_generation: 1,
            source_head_digest: source.clone(),
            dependency_lock_digest: Some(lock.clone()),
            updated_at: 7,
        })
        .await
        .unwrap();
    let (second_candidate, _, second_artifact) = add_managed_candidate(
        &fixture,
        2,
        &source,
        &lock,
        'b',
        "1.1.0",
        Some(fixture.artifact_digest.clone()),
        8,
    )
    .await;
    let replaced = fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id.clone(),
            candidate_id: second_candidate,
            expected_project_generation: 2,
            expected_mount_revision: Some(1),
            expected_current_artifact_digest: Some(fixture.artifact_digest.clone()),
            new_mount_id: None,
            new_data_dir_path: None,
            applied_at: 11,
        })
        .await
        .unwrap();
    assert_eq!(
        replaced.current_artifact_digest.as_deref(),
        Some(second_artifact.as_str())
    );
    assert_eq!(
        replaced.previous_artifact_digest.as_deref(),
        Some(fixture.artifact_digest.as_str())
    );

    let next_source = digest('7');
    let next_lock = digest('8');
    fixture
        .repo
        .update_project_source_cas(&UpdatePluginProjectSourceParams {
            project_id: fixture.project_id.clone(),
            expected_generation: 2,
            source_head_digest: next_source.clone(),
            dependency_lock_digest: Some(next_lock.clone()),
            updated_at: 12,
        })
        .await
        .unwrap();
    let (stale_candidate, _, _) = add_managed_candidate(
        &fixture,
        3,
        &next_source,
        &next_lock,
        'c',
        "1.2.0",
        Some(fixture.artifact_digest.clone()),
        13,
    )
    .await;
    let stale = fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id.clone(),
            candidate_id: stale_candidate,
            expected_project_generation: 3,
            expected_mount_revision: Some(2),
            expected_current_artifact_digest: Some(second_artifact.clone()),
            new_mount_id: None,
            new_data_dir_path: None,
            applied_at: 16,
        })
        .await
        .unwrap_err();
    assert!(matches!(stale, DbError::Conflict(message) if message.contains("base target")));

    let restored = fixture
        .repo
        .restore_previous(&RestorePluginMountParams {
            mount_id: mount_id.clone(),
            expected_revision: 2,
            expected_current_artifact_digest: second_artifact.clone(),
            expected_previous_artifact_digest: fixture.artifact_digest.clone(),
            restored_at: 17,
        })
        .await
        .unwrap();
    assert_eq!(restored.revision, 3);
    assert_eq!(
        restored.current_artifact_digest.as_deref(),
        Some(fixture.artifact_digest.as_str())
    );
    assert!(fixture
        .repo
        .restore_previous(&RestorePluginMountParams {
            mount_id,
            expected_revision: 2,
            expected_current_artifact_digest: second_artifact,
            expected_previous_artifact_digest: fixture.artifact_digest,
            restored_at: 18,
        })
        .await
        .is_err());
}

#[tokio::test]
async fn direct_current_pointer_sql_bypass_is_rejected() {
    let fixture = managed_fixture().await;
    let mount_id = id();
    fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id,
            candidate_id: fixture.candidate_id,
            expected_project_generation: 1,
            expected_mount_revision: Some(0),
            expected_current_artifact_digest: None,
            new_mount_id: Some(mount_id.clone()),
            new_data_dir_path: Some(format!("plugin-data/{mount_id}")),
            applied_at: 6,
        })
        .await
        .unwrap();
    let invalid = sqlx::query(
        "UPDATE plugin_mounts
         SET current_artifact_digest = ?, current_revision_id = ?, revision = revision + 1
         WHERE mount_id = ?",
    )
    .bind(digest('f'))
    .bind(id())
    .bind(&mount_id)
    .execute(&fixture.pool)
    .await;
    assert!(invalid.is_err());
    let unchanged = fixture.repo.get_mount(&mount_id).await.unwrap().unwrap();
    assert_eq!(
        unchanged.current_artifact_digest.as_deref(),
        Some(fixture.artifact_digest.as_str())
    );
}

#[tokio::test]
async fn candidate_test_receipt_is_exact_and_one_to_one() {
    let fixture = managed_fixture().await;
    let receipt = RecordPluginCandidateTestReceiptParams {
        receipt_id: id(),
        candidate_id: fixture.candidate_id,
        candidate_digest: digest('4'),
        artifact_id: fixture.artifact_id,
        artifact_digest: fixture.artifact_digest,
        receipt_digest: digest('5'),
        runtime_fingerprint_digest: digest('6'),
        receipt: json!({"result": "passed"}),
        tested_at: 6,
    };
    fixture
        .repo
        .record_candidate_test_receipt(&receipt)
        .await
        .unwrap();
    assert!(fixture
        .repo
        .record_candidate_test_receipt(&RecordPluginCandidateTestReceiptParams {
            receipt_id: id(),
            receipt_digest: digest('7'),
            ..receipt.clone()
        })
        .await
        .is_err());
    assert!(fixture
        .repo
        .record_candidate_test_receipt(&RecordPluginCandidateTestReceiptParams {
            receipt_id: id(),
            candidate_id: id(),
            receipt_digest: digest('8'),
            ..receipt
        })
        .await
        .is_err());
}

#[tokio::test]
async fn mount_data_delete_preserves_project_ready_candidate_and_test_receipt_as_stale() {
    let fixture = managed_fixture().await;
    let mount_id = id();
    let installed = fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id.clone(),
            candidate_id: fixture.candidate_id.clone(),
            expected_project_generation: 1,
            expected_mount_revision: Some(0),
            expected_current_artifact_digest: None,
            new_mount_id: Some(mount_id.clone()),
            new_data_dir_path: Some(format!("plugin-data/{mount_id}")),
            applied_at: 6,
        })
        .await
        .unwrap();
    let next_source = digest('5');
    let next_lock = digest('6');
    fixture
        .repo
        .update_project_source_cas(&UpdatePluginProjectSourceParams {
            project_id: fixture.project_id.clone(),
            expected_generation: 1,
            source_head_digest: next_source.clone(),
            dependency_lock_digest: Some(next_lock.clone()),
            updated_at: 7,
        })
        .await
        .unwrap();
    let (ready_candidate_id, ready_artifact_id, ready_artifact_digest) = add_managed_candidate(
        &fixture,
        2,
        &next_source,
        &next_lock,
        'b',
        "1.1.0",
        Some(fixture.artifact_digest.clone()),
        8,
    )
    .await;
    let receipt_id = id();
    fixture
        .repo
        .record_candidate_test_receipt(&RecordPluginCandidateTestReceiptParams {
            receipt_id: receipt_id.clone(),
            candidate_id: ready_candidate_id.clone(),
            candidate_digest: digest('9'),
            artifact_id: ready_artifact_id,
            artifact_digest: ready_artifact_digest,
            receipt_digest: digest('a'),
            runtime_fingerprint_digest: digest('b'),
            receipt: json!({"result": "passed"}),
            tested_at: 11,
        })
        .await
        .unwrap();
    fixture
        .repo
        .bind_credential(&mount_id, "api_token", "credential-store:item-1", 11)
        .await
        .unwrap();
    fixture
        .repo
        .put_kv_cas(&PutPluginKvParams {
            mount_id: mount_id.clone(),
            namespace: "runtime".into(),
            key: "counter".into(),
            value: json!(1),
            expected_revision: None,
            updated_at: 11,
        })
        .await
        .unwrap();
    let retained = fixture
        .repo
        .uninstall_retain_data(&UninstallPluginMountParams {
            mount_id: mount_id.clone(),
            expected_revision: installed.revision,
            expected_current_artifact_digest: installed.current_artifact_digest.unwrap(),
            uninstalled_at: 12,
        })
        .await
        .unwrap();
    let pending = fixture
        .repo
        .mark_mount_delete_pending(&mount_id, retained.revision, 13)
        .await
        .unwrap();
    let replay = fixture
        .repo
        .mark_mount_delete_pending(&mount_id, retained.revision, 14)
        .await
        .unwrap();
    assert_eq!(replay.revision, pending.revision);
    assert!(fixture.repo.complete_mount_data_delete(&mount_id).await.unwrap());
    assert!(!fixture.repo.complete_mount_data_delete(&mount_id).await.unwrap());
    assert!(fixture.repo.get_mount(&mount_id).await.unwrap().is_none());

    let project = fixture
        .repo
        .get_project(&fixture.project_id)
        .await
        .unwrap()
        .unwrap();
    assert!(project.linked_mount_id.is_none());
    assert_eq!(project.build_generation, 3);
    assert_eq!(
        project.ready_candidate_id.as_deref(),
        Some(ready_candidate_id.as_str())
    );
    assert_eq!(project.source_head_digest.as_deref(), Some(next_source.as_str()));
    assert_eq!(
        project.dependency_lock_digest.as_deref(),
        Some(next_lock.as_str())
    );
    let ready = fixture
        .repo
        .get_ready_candidate(&fixture.project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready.candidate_id, ready_candidate_id);
    assert_eq!(ready.build_generation, 2, "generation advance makes it stale");
    let receipt_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM plugin_candidate_test_receipts
            WHERE receipt_id = ? AND candidate_id = ?
         )",
    )
    .bind(receipt_id)
    .bind(&ready.candidate_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert!(receipt_exists);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM plugin_credential_bindings WHERE mount_id = ?",
        )
        .bind(&mount_id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM plugin_kv WHERE mount_id = ?")
            .bind(&mount_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn kv_is_mount_namespaced_and_uses_revision_cas() {
    let fixture = managed_fixture().await;
    let mount_id = id();
    fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id,
            candidate_id: fixture.candidate_id,
            expected_project_generation: 1,
            expected_mount_revision: Some(0),
            expected_current_artifact_digest: None,
            new_mount_id: Some(mount_id.clone()),
            new_data_dir_path: Some(format!("plugin-data/{mount_id}")),
            applied_at: 6,
        })
        .await
        .unwrap();
    let first = fixture
        .repo
        .put_kv_cas(&PutPluginKvParams {
            mount_id: mount_id.clone(),
            namespace: "runtime".into(),
            key: "state".into(),
            value: json!({"value": 1}),
            expected_revision: None,
            updated_at: 7,
        })
        .await
        .unwrap();
    assert_eq!(first.revision, 1);
    fixture
        .repo
        .put_kv_cas(&PutPluginKvParams {
            mount_id: mount_id.clone(),
            namespace: "preview".into(),
            key: "state".into(),
            value: json!({"value": 99}),
            expected_revision: None,
            updated_at: 7,
        })
        .await
        .unwrap();
    let second = fixture
        .repo
        .put_kv_cas(&PutPluginKvParams {
            mount_id: mount_id.clone(),
            namespace: "runtime".into(),
            key: "state".into(),
            value: json!({"value": 2}),
            expected_revision: Some(1),
            updated_at: 8,
        })
        .await
        .unwrap();
    assert_eq!(second.revision, 2);
    assert!(fixture
        .repo
        .put_kv_cas(&PutPluginKvParams {
            mount_id,
            namespace: "runtime".into(),
            key: "state".into(),
            value: json!({"value": 3}),
            expected_revision: Some(1),
            updated_at: 9,
        })
        .await
        .is_err());
}

#[tokio::test]
async fn product_operation_owner_state_progress_error_and_log_contract_is_strict() {
    let fixture = managed_fixture().await;
    let mount_id = id();
    fixture
        .repo
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: fixture.project_id.clone(),
            candidate_id: fixture.candidate_id,
            expected_project_generation: 1,
            expected_mount_revision: Some(0),
            expected_current_artifact_digest: None,
            new_mount_id: Some(mount_id.clone()),
            new_data_dir_path: Some(format!("plugin-data/{mount_id}")),
            applied_at: 6,
        })
        .await
        .unwrap();

    assert!(fixture
        .repo
        .start_operation(&StartProductOperationParams {
            operation_id: id(),
            kind: ProductOperationKind::Build,
            owner_kind: "plugin_mount".into(),
            owner_id: mount_id.clone(),
            progress_percent: Some(0),
            bounded_log_tail: vec![],
            started_at_ms: 7,
        })
        .await
        .is_err());
    let import = fixture
        .repo
        .start_operation(&StartProductOperationParams {
            operation_id: id(),
            kind: ProductOperationKind::Import,
            owner_kind: "plugin_mount".into(),
            owner_id: mount_id,
            progress_percent: None,
            bounded_log_tail: vec!["importing".into()],
            started_at_ms: 7,
        })
        .await
        .unwrap();
    assert_eq!(import.owner_kind, "plugin_mount");
    assert_eq!(import.progress_percent, None);
    assert_eq!(import.bounded_log_tail_json, r#"["importing"]"#);

    let failed = fixture
        .repo
        .finish_operation(&FinishProductOperationParams {
            operation_id: import.operation_id,
            state: ProductOperationState::Failed,
            progress_percent: Some(42),
            last_error_code: Some("plugin_import_failed".into()),
            bounded_log_tail: vec!["importing".into(), "failed".into()],
            finished_at_ms: 8,
        })
        .await
        .unwrap();
    assert_eq!(failed.last_error_code.as_deref(), Some("plugin_import_failed"));

    let too_many_lines = vec!["line".to_owned(); MAX_PRODUCT_OPERATION_LOG_LINES + 1];
    assert!(fixture
        .repo
        .start_operation(&StartProductOperationParams {
            operation_id: id(),
            kind: ProductOperationKind::Export,
            owner_kind: "plugin_project".into(),
            owner_id: fixture.project_id.clone(),
            progress_percent: Some(0),
            bounded_log_tail: too_many_lines,
            started_at_ms: 9,
        })
        .await
        .is_err());
    assert!(fixture
        .repo
        .start_operation(&StartProductOperationParams {
            operation_id: id(),
            kind: ProductOperationKind::Export,
            owner_kind: "plugin_project".into(),
            owner_id: fixture.project_id,
            progress_percent: Some(0),
            bounded_log_tail: vec!["x".repeat(MAX_PRODUCT_OPERATION_LOG_LINE_CHARS + 1)],
            started_at_ms: 9,
        })
        .await
        .is_err());

    let invalid_matrix = sqlx::query(
        "INSERT INTO product_operations (
            operation_id, kind, owner_kind, owner_id, state, progress_percent,
            bounded_log_tail_json, started_at_ms
         ) VALUES (?, 'build', 'plugin_mount', ?, 'running', 0, '[]', 1)",
    )
    .bind(id())
    .bind(id())
    .execute(&fixture.pool)
    .await;
    assert!(invalid_matrix.is_err());
    let invalid_log = sqlx::query(
        "INSERT INTO product_operations (
            operation_id, kind, owner_kind, owner_id, state, progress_percent,
            bounded_log_tail_json, started_at_ms
         ) VALUES (?, 'export', 'miniapp', ?, 'running', 0, '[1]', 1)",
    )
    .bind(id())
    .bind(id())
    .execute(&fixture.pool)
    .await;
    assert!(invalid_log.is_err());
}
