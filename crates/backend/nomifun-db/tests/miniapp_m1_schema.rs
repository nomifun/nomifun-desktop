use nomifun_agent_contracts::MINIAPP_RELEASE_PROFILE_VERSION;
use nomifun_db::{
    init_database, installation_owner_id, validate_id_data_contract,
};
use serde_json::json;
use sqlx::Row;
use sqlx::migrate::{Migrate, Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn migrate_through(pool: &nomifun_db::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= maximum_version)
    {
        connection.apply(migration).await.unwrap();
    }
}

async fn migrated_pool(maximum_version: i64) -> nomifun_db::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&pool, maximum_version).await;
    let owner = "0190f5fe-7c00-7000-8000-000000000201";
    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, 'admin', '', '', 1, 1)",
    )
    .bind(owner)
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
    pool
}

#[allow(clippy::too_many_arguments)]
async fn insert_succeeded_miniapp_build(
    pool: &nomifun_db::SqlitePool,
    owner: &str,
    miniapp_id: &str,
    project_id: &str,
    operation_id: &str,
    project_revision: i64,
    source_snapshot_digest: &str,
    dependency_lock_digest: &str,
    build_generation: i64,
    started_at_ms: i64,
    finished_at_ms: i64,
) {
    sqlx::query(
        "INSERT INTO product_operations (
            operation_id, kind, owner_kind, owner_id, state,
            progress_percent, bounded_log_tail_json,
            started_at_ms, finished_at_ms
         ) VALUES (?, 'build', 'miniapp', ?, 'succeeded', 100, '[]', ?, ?)",
    )
    .bind(operation_id)
    .bind(miniapp_id)
    .bind(started_at_ms)
    .bind(finished_at_ms)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_build_operation_lineage (
            operation_id, owner_user_id, miniapp_id, project_id,
            project_revision, source_snapshot_digest,
            dependency_lock_digest, build_profile_version,
            build_generation, started_at_ms
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(operation_id)
    .bind(owner)
    .bind(miniapp_id)
    .bind(project_id)
    .bind(project_revision)
    .bind(source_snapshot_digest)
    .bind(dependency_lock_digest)
    .bind(MINIAPP_RELEASE_PROFILE_VERSION)
    .bind(build_generation)
    .bind(started_at_ms)
    .execute(pool)
    .await
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn release_record_json(
    miniapp_id: &str,
    project_id: &str,
    artifact_id: &str,
    release_id: &str,
    artifact_digest: &str,
    manifest_digest: &str,
    operation_id: &str,
    source_snapshot_digest: &str,
    dependency_lock_digest: &str,
    build_generation: i64,
    created_at_ms: i64,
) -> String {
    json!({
        "miniapp_id": miniapp_id,
        "release": {
            "release_id": release_id,
            "artifact_id": artifact_id,
            "release_digest": artifact_digest,
            "manifest_digest": manifest_digest
        },
        "origin_operation_id": operation_id,
        "origin": "build",
        "source_lineage": {
            "kind": "managed",
            "project_id": project_id,
            "source_snapshot_digest": source_snapshot_digest,
            "dependency_lock_digest": dependency_lock_digest,
            "build_profile_version": MINIAPP_RELEASE_PROFILE_VERSION,
            "build_generation": build_generation
        },
        "created_at_ms": created_at_ms
    })
    .to_string()
}

#[tokio::test]
async fn migration_072_is_additive_and_starts_with_an_empty_new_root() {
    let pool = migrated_pool(75).await;

    let old_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapps")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(old_count, 0);

    for table in [
        "miniapp_library_state",
        "miniapp_products",
        "miniapp_projects",
        "miniapp_release_artifacts",
        "miniapp_releases",
        "miniapp_credential_bindings",
        "miniapp_kv",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "new M1 table {table} must start empty");
    }

    let migration = include_str!("../migrations/072_miniapp_m1_data_root.sql");
    assert!(!migration.contains("FROM miniapps"));
    assert!(!migration.contains("INSERT INTO miniapps"));
    assert!(!migration.contains("UPDATE miniapps"));
}

#[tokio::test]
async fn new_root_preserves_owner_and_pointer_shape_checks() {
    let database = migrated_pool(75).await;
    let owner = installation_owner_id(&database).await.unwrap();
    let miniapp_id = "0190f5fe-7c00-7000-8000-000000000001";
    let invalid_miniapp_id = "not-a-uuid";

    let invalid = sqlx::query(
        "INSERT INTO miniapp_products (
            miniapp_id, owner_user_id, display_name, kind,
            materialized_catalog_digest, created_at, updated_at
         ) VALUES (?, ?, 'Invalid', 'ui_only', ?, 1, 1)",
    )
    .bind(invalid_miniapp_id)
    .bind(&owner)
    .bind("a".repeat(64))
    .execute(&database)
    .await;
    assert!(invalid.is_err());

    sqlx::query(
        "INSERT INTO miniapp_products (
            miniapp_id, owner_user_id, display_name, kind,
            materialized_catalog_digest, created_at, updated_at
         ) VALUES (?, ?, 'Valid', 'ui_only', ?, 1, 1)",
    )
    .bind(miniapp_id)
    .bind(&owner)
    .bind("a".repeat(64))
    .execute(&database)
    .await
    .unwrap();

    let row = sqlx::query(
        "SELECT miniapp_id, owner_user_id, lifecycle, active_release_epoch
         FROM miniapp_products WHERE miniapp_id = ?",
    )
    .bind(miniapp_id)
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("miniapp_id"), miniapp_id);
    assert_eq!(row.get::<String, _>("owner_user_id"), owner);
    assert_eq!(row.get::<String, _>("lifecycle"), "disabled");
    assert_eq!(row.get::<i64, _>("active_release_epoch"), 0);
}

#[tokio::test]
async fn migration_072_preserves_existing_legacy_miniapp_rows_byte_for_byte() {
    let database = nomifun_db::SqlitePool::connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&database, 71).await;
    let owner = "0190f5fe-7c00-7000-8000-000000000201";
    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(owner)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    let legacy_id = "0190f5fe-7c00-7000-8000-000000000202";
    sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size,
            created_at, updated_at
         ) VALUES (?, ?, 'legacy', 'old', '<p>old</p>', 10, 2, 3)",
    )
    .bind(legacy_id)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    let before: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT name, description, html, created_at, updated_at
         FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&database)
    .await
    .unwrap();

    let mut connection = database.acquire().await.unwrap();
    let migration = MIGRATOR
        .iter()
        .find(|migration| migration.version == 72)
        .unwrap();
    connection.apply(migration).await.unwrap();
    drop(connection);

    let after: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT name, description, html, created_at, updated_at
         FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(after, before);
    let new_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_products")
            .fetch_one(&database)
            .await
            .unwrap();
    assert_eq!(new_count, 0);
}

#[tokio::test]
async fn migration_075_adds_only_owner_scoped_host_kv_without_runtime_sidecars() {
    let database = migrated_pool(75).await;
    let migration = include_str!("../migrations/075_miniapp_m1_runtime_state.sql");
    for forbidden in [
        "FROM miniapps",
        "INSERT INTO miniapps",
        "UPDATE miniapps",
        "DELETE FROM miniapps",
        "FOREIGN KEY",
        "REFERENCES",
        "CREATE TRIGGER",
        "files_dir",
        "private_database",
        "service_host",
    ] {
        assert!(
            !migration.contains(forbidden),
            "migration 075 must not contain {forbidden}"
        );
    }

    let columns = sqlx::query("PRAGMA table_info('miniapp_kv')")
        .fetch_all(&database)
        .await
        .unwrap();
    let names = columns
        .iter()
        .map(|column| column.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "id",
            "miniapp_id",
            "owner_user_id",
            "namespace",
            "key",
            "value_json",
            "revision",
            "created_at",
            "updated_at",
        ]
    );
    let foreign_keys: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('miniapp_kv')",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(foreign_keys, 0);
    let triggers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'trigger' AND tbl_name = 'miniapp_kv'",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(triggers, 0);
}

#[tokio::test]
async fn migration_075_preserves_existing_072_and_legacy_rows_byte_for_byte() {
    let database = nomifun_db::SqlitePool::connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&database, 74).await;
    let owner = "0190f5fe-7c00-7000-8000-000000000201";
    let miniapp_id = "0190f5fe-7c00-7000-8000-000000000202";
    let project_id = "0190f5fe-7c00-7000-8000-000000000203";
    let legacy_id = "0190f5fe-7c00-7000-8000-000000000204";
    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(owner)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size,
            created_at, updated_at
         ) VALUES (?, ?, 'legacy', 'old', '<p>old</p>', 10, 2, 3)",
    )
    .bind(legacy_id)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_library_state
         (singleton_key, owner_user_id, revision, updated_at)
         VALUES ('miniapp_m1', ?, 1, 10)",
    )
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_products (
            miniapp_id, owner_user_id, display_name, kind,
            materialized_catalog_digest, config_schema_json, config_json,
            created_at, updated_at
         ) VALUES (?, ?, 'M1', 'ui_only', ?, ?, ?, 10, 10)",
    )
    .bind(miniapp_id)
    .bind(owner)
    .bind("a".repeat(64))
    .bind(r#"{"type":"object"}"#)
    .bind(r#"{"theme":"dark"}"#)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_projects
         (project_id, miniapp_id, owner_user_id, created_at, updated_at)
         VALUES (?, ?, ?, 10, 10)",
    )
    .bind(project_id)
    .bind(miniapp_id)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_credential_bindings
         (miniapp_id, owner_user_id, slot_key, credential_id, created_at, updated_at)
         VALUES (?, ?, 'primary', 'credential-primary', 10, 10)",
    )
    .bind(miniapp_id)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();

    let legacy_before: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT name, description, html, created_at, updated_at
         FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&database)
    .await
    .unwrap();
    let product_before: (i64, String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT product_revision, config_schema_json, config_json,
                config_revision, credential_bindings_revision, updated_at
         FROM miniapp_products WHERE miniapp_id = ?",
    )
    .bind(miniapp_id)
    .fetch_one(&database)
    .await
    .unwrap();
    let binding_before: (String, String, i64, i64) = sqlx::query_as(
        "SELECT slot_key, credential_id, created_at, updated_at
         FROM miniapp_credential_bindings WHERE miniapp_id = ?",
    )
    .bind(miniapp_id)
    .fetch_one(&database)
    .await
    .unwrap();

    let mut connection = database.acquire().await.unwrap();
    connection
        .apply(MIGRATOR.iter().find(|migration| migration.version == 75).unwrap())
        .await
        .unwrap();
    drop(connection);

    let legacy_after: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT name, description, html, created_at, updated_at
         FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&database)
    .await
    .unwrap();
    let product_after: (i64, String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT product_revision, config_schema_json, config_json,
                config_revision, credential_bindings_revision, updated_at
         FROM miniapp_products WHERE miniapp_id = ?",
    )
    .bind(miniapp_id)
    .fetch_one(&database)
    .await
    .unwrap();
    let binding_after: (String, String, i64, i64) = sqlx::query_as(
        "SELECT slot_key, credential_id, created_at, updated_at
         FROM miniapp_credential_bindings WHERE miniapp_id = ?",
    )
    .bind(miniapp_id)
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(legacy_after, legacy_before);
    assert_eq!(product_after, product_before);
    assert_eq!(binding_after, binding_before);
    let kv_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_kv")
        .fetch_one(&database)
        .await
        .unwrap();
    assert_eq!(kv_count, 0);
}

#[tokio::test]
async fn migration_077_adds_only_immutable_miniapp_build_lineage() {
    let database = migrated_pool(77).await;
    let migration = include_str!("../migrations/077_miniapp_build_operation_lineage.sql");
    for forbidden in [
        "FROM miniapps",
        "INSERT INTO miniapps",
        "UPDATE miniapps",
        "DELETE FROM miniapps",
        "FOREIGN KEY",
        "REFERENCES",
        "CREATE TRIGGER",
        "service_host",
        "private_database",
    ] {
        assert!(
            !migration.contains(forbidden),
            "migration 077 must not contain {forbidden}"
        );
    }

    let columns = sqlx::query("PRAGMA table_info('miniapp_build_operation_lineage')")
        .fetch_all(&database)
        .await
        .unwrap();
    let names = columns
        .iter()
        .map(|column| column.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "id",
            "operation_id",
            "owner_user_id",
            "miniapp_id",
            "project_id",
            "project_revision",
            "source_snapshot_digest",
            "dependency_lock_digest",
            "build_profile_version",
            "build_generation",
            "started_at_ms",
        ]
    );
    let foreign_keys: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('miniapp_build_operation_lineage')",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(foreign_keys, 0);
}

#[tokio::test]
async fn migration_078_adds_authorization_and_catalog_projection_tables() {
    let database = migrated_pool(78).await;
    let migration = include_str!("../migrations/078_miniapp_publish_authorizations.sql");
    assert!(!migration.contains("FROM miniapps"));
    assert!(!migration.contains("FOREIGN KEY"));
    assert!(!migration.contains("CREATE TRIGGER"));

    for table in [
        "miniapp_publish_authorizations",
        "miniapp_catalog_publications",
    ] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(&database)
        .await
        .unwrap();
        assert_eq!(count, 1, "migration 078 must create {table}");
        let foreign_keys: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('{table}')"
        ))
        .fetch_one(&database)
        .await
        .unwrap();
        assert_eq!(foreign_keys, 0, "{table} must not have physical FKs");
    }
}

#[tokio::test]
async fn migration_079_allows_release_content_reuse_without_weakening_release_identity() {
    let database = migrated_pool(79).await;
    let migration = include_str!("../migrations/079_miniapp_release_digest_reuse.sql");
    assert!(!migration.contains("FROM miniapps"));
    assert!(!migration.contains("FOREIGN KEY"));
    assert!(!migration.contains("CREATE TRIGGER"));

    let indexes = sqlx::query("PRAGMA index_list('miniapp_releases')")
        .fetch_all(&database)
        .await
        .unwrap();
    let mut unique_columns = Vec::new();
    for index in indexes {
        if index.get::<i64, _>("unique") == 0 {
            continue;
        }
        let name = index.get::<String, _>("name");
        let columns = sqlx::query(&format!("PRAGMA index_info('{name}')"))
            .fetch_all(&database)
            .await
            .unwrap()
            .into_iter()
            .map(|column| column.get::<String, _>("name"))
            .collect::<Vec<_>>();
        unique_columns.push(columns);
    }
    assert!(
        !unique_columns
            .iter()
            .any(|columns| columns == &["release_digest"])
    );
    assert!(
        !unique_columns
            .iter()
            .any(|columns| columns == &["owner_user_id", "release_digest"])
    );
    assert!(
        unique_columns
            .iter()
            .any(|columns| columns == &["release_id"])
    );
}

#[tokio::test]
async fn migration_080_adds_digest_only_surface_session_authority() {
    let database = migrated_pool(80).await;
    let migration = include_str!("../migrations/080_miniapp_surface_sessions.sql");
    assert!(!migration.contains("FROM miniapps"));
    assert!(!migration.contains("FOREIGN KEY"));
    assert!(!migration.contains("CREATE TRIGGER"));
    assert!(!migration.contains("raw_capability"));

    let columns = sqlx::query("PRAGMA table_info('miniapp_surface_sessions')")
        .fetch_all(&database)
        .await
        .unwrap()
        .into_iter()
        .map(|column| column.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "id",
            "surface_session_id",
            "miniapp_id",
            "owner_user_id",
            "generation",
            "capability_digest",
            "active_release_id",
            "active_release_digest",
            "active_release_epoch",
            "issued_at_ms",
        ]
    );
    assert!(
        !columns.iter().any(|column| column == "capability"),
        "raw Surface capabilities must not be persisted"
    );
}

#[tokio::test]
async fn migration_081_adds_monotonic_host_kv_tombstone_columns() {
    let database = migrated_pool(81).await;
    let migration = include_str!("../migrations/081_miniapp_kv_tombstones.sql");
    assert!(!migration.contains("FROM miniapps"));
    assert!(!migration.contains("FOREIGN KEY"));
    assert!(!migration.contains("CREATE TRIGGER"));

    let columns = sqlx::query("PRAGMA table_info('miniapp_kv')")
        .fetch_all(&database)
        .await
        .unwrap()
        .into_iter()
        .map(|column| column.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "id",
            "miniapp_id",
            "owner_user_id",
            "namespace",
            "key",
            "value_json",
            "revision",
            "created_at",
            "updated_at",
            "key_generation",
            "is_tombstone",
        ]
    );
    for (column, expected_default) in [
        ("key_generation", "1"),
        ("is_tombstone", "0"),
    ] {
        let default_value: Option<String> = sqlx::query_scalar(&format!(
            "SELECT dflt_value FROM pragma_table_info('miniapp_kv') WHERE name = ?"
        ))
        .bind(column)
        .fetch_optional(&database)
        .await
        .unwrap()
        .flatten();
        assert_eq!(default_value.as_deref(), Some(expected_default));
    }
}

#[tokio::test]
async fn migration_082_adds_owner_scoped_miniapp_deletion_intents() {
    let database = migrated_pool(82).await;
    let migration = include_str!("../migrations/082_miniapp_deletion_intents.sql");
    for forbidden in ["FROM miniapps", "FOREIGN KEY", "REFERENCES", "CREATE TRIGGER"] {
        assert!(
            !migration.contains(forbidden),
            "migration 082 must not contain {forbidden}"
        );
    }

    let columns = sqlx::query("PRAGMA table_info('miniapp_deletion_intents')")
        .fetch_all(&database)
        .await
        .unwrap()
        .into_iter()
        .map(|column| column.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "id",
            "miniapp_id",
            "owner_user_id",
            "operation_id",
            "started_at_ms",
            "last_error_code",
        ]
    );
    let foreign_keys: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('miniapp_deletion_intents')",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(foreign_keys, 0);
    let triggers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'trigger' AND tbl_name = 'miniapp_deletion_intents'",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(triggers, 0);
    for index in [
        "idx_miniapp_deletion_intents_owner_user_id",
        "idx_miniapp_deletion_intents_miniapp_id",
        "idx_miniapp_deletion_intents_operation_id",
    ] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name = ?",
        )
        .bind(index)
        .fetch_one(&database)
        .await
        .unwrap();
        assert_eq!(count, 1, "missing index {index}");
    }
}

#[tokio::test]
async fn migration_083_adds_immutable_owner_scoped_service_test_receipt_history() {
    let database = migrated_pool(83).await;
    let migration = include_str!("../migrations/083_miniapp_service_test_receipts.sql");
    for forbidden in ["FROM miniapps", "FOREIGN KEY", "REFERENCES", "CREATE TRIGGER"] {
        assert!(
            !migration.contains(forbidden),
            "migration 083 must not contain {forbidden}"
        );
    }

    let columns = sqlx::query("PRAGMA table_info('miniapp_service_test_receipts')")
        .fetch_all(&database)
        .await
        .unwrap()
        .into_iter()
        .map(|column| column.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "id",
            "receipt_id",
            "owner_user_id",
            "miniapp_id",
            "release_id",
            "release_digest",
            "service_run_key",
            "outcome",
            "error_code",
            "receipt_digest",
            "runtime_fingerprint_digest",
            "resolved_test_input_digest",
            "tested_product_revision",
            "tested_pointer_revision",
            "tested_config_revision",
            "tested_credential_bindings_revision",
            "receipt_json",
            "issued_at_ms",
        ]
    );
    let foreign_keys: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('miniapp_service_test_receipts')",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(foreign_keys, 0);
    let triggers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'trigger' AND tbl_name = 'miniapp_service_test_receipts'",
    )
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(triggers, 0);
    for index in [
        "idx_miniapp_service_test_receipts_owner_user_id",
        "idx_miniapp_service_test_receipts_miniapp_id",
        "idx_miniapp_service_test_receipts_release_id",
        "idx_miniapp_service_test_receipts_current",
    ] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name = ?",
        )
        .bind(index)
        .fetch_one(&database)
        .await
        .unwrap();
        assert_eq!(count, 1, "missing index {index}");
    }

    let invalid_receipt_id = sqlx::query(
        "INSERT INTO miniapp_service_test_receipts (
            receipt_id, owner_user_id, miniapp_id, release_id,
            release_digest, service_run_key, outcome, receipt_digest,
            runtime_fingerprint_digest, resolved_test_input_digest,
            tested_product_revision, tested_pointer_revision,
            tested_config_revision, tested_credential_bindings_revision,
            receipt_json, issued_at_ms
         ) VALUES ('not-a-uuid', ?, ?, ?, ?, ?, 'passed', ?, ?, ?, 1, 1, 1, 1, '{}', 1)",
    )
    .bind("0190f5fe-7c00-7000-8000-000000000201")
    .bind("0190f5fe-7c00-7000-8000-000000000301")
    .bind("0190f5fe-7c00-7000-8000-000000000302")
    .bind("a".repeat(64))
    .bind("b".repeat(64))
    .bind("c".repeat(64))
    .bind("d".repeat(64))
    .bind("e".repeat(64))
    .execute(&database)
    .await;
    assert!(invalid_receipt_id.is_err());

    let invalid_outcome = sqlx::query(
        "INSERT INTO miniapp_service_test_receipts (
            receipt_id, owner_user_id, miniapp_id, release_id,
            release_digest, service_run_key, outcome, receipt_digest,
            runtime_fingerprint_digest, resolved_test_input_digest,
            tested_product_revision, tested_pointer_revision,
            tested_config_revision, tested_credential_bindings_revision,
            receipt_json, issued_at_ms
         ) VALUES (?, ?, ?, ?, ?, ?, 'unknown', ?, ?, ?, 1, 1, 1, 1, '{}', 1)",
    )
    .bind("0190f5fe-7c00-7000-8000-000000000303")
    .bind("0190f5fe-7c00-7000-8000-000000000201")
    .bind("0190f5fe-7c00-7000-8000-000000000301")
    .bind("0190f5fe-7c00-7000-8000-000000000302")
    .bind("a".repeat(64))
    .bind("b".repeat(64))
    .bind("c".repeat(64))
    .bind("d".repeat(64))
    .bind("e".repeat(64))
    .execute(&database)
    .await;
    assert!(invalid_outcome.is_err());
}

#[tokio::test]
async fn migrations_078_through_083_upgrade_existing_release_state_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("miniapp-v077-upgrade.db");
    let database = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    migrate_through(&database, 77).await;

    let owner = "0190f5fe-7c00-7000-8000-000000000201";
    let miniapp_id = "0190f5fe-7c00-7000-8000-000000000301";
    let project_id = "0190f5fe-7c00-7000-8000-000000000302";
    let artifact_id = "0190f5fe-7c00-7000-8000-000000000303";
    let release_id = "0190f5fe-7c00-7000-8000-000000000304";
    let operation_id = "0190f5fe-7c00-7000-8000-000000000305";
    let reused_release_id = "0190f5fe-7c00-7000-8000-000000000306";
    let reused_operation_id = "0190f5fe-7c00-7000-8000-000000000307";
    let retired_operation_id = "0190f5fe-7c00-7000-8000-000000000308";
    let retired_artifact_id = "0190f5fe-7c00-7000-8000-000000000309";
    let retired_release_id = "0190f5fe-7c00-7000-8000-00000000030a";
    let artifact_digest = "a".repeat(64);
    let manifest_digest = "b".repeat(64);
    let source_digest = "c".repeat(64);
    let reused_source_digest = "4".repeat(64);
    let dependency_digest = "e".repeat(64);
    let catalog_digest = "f".repeat(64);
    let retired_artifact_digest = "1".repeat(64);
    let retired_manifest_digest = "2".repeat(64);
    let retired_source_digest = "3".repeat(64);
    let old_sequence = 100_i64;

    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, 'admin', '', '', 1, 1)",
    )
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO installation_identity (singleton_key, owner_user_id)
         VALUES ('installation', ?)",
    )
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO miniapp_library_state
         (singleton_key, owner_user_id, revision, updated_at)
         VALUES ('miniapp_m1', ?, 4, 40)",
    )
    .bind(&owner)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_products (
            miniapp_id, owner_user_id, product_revision, display_name, kind,
            lifecycle, pointer_revision, active_release_epoch,
            active_release_id, active_release_digest, materialized_catalog_digest,
            created_at, updated_at
         ) VALUES (?, ?, 4, 'Upgraded App', 'ui_only', 'enabled', 4, 1,
                   ?, ?, ?, 10, 40)",
    )
    .bind(miniapp_id)
    .bind(owner)
    .bind(release_id)
    .bind(&artifact_digest)
    .bind(&catalog_digest)
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_projects (
            project_id, miniapp_id, owner_user_id, project_revision,
            source_state, managed_source_path, source_head_digest,
            dependency_lock_digest, build_profile_version, build_generation,
            created_at, updated_at
         ) VALUES (?, ?, ?, 2, 'editable', ?, ?, ?, ?, 2, 10, 40)",
    )
    .bind(project_id)
    .bind(miniapp_id)
    .bind(owner)
    .bind("sources/owner/miniapp/project")
    .bind(&retired_source_digest)
    .bind(&dependency_digest)
    .bind(MINIAPP_RELEASE_PROFILE_VERSION)
    .execute(&database)
    .await
    .unwrap();
    insert_succeeded_miniapp_build(
        &database,
        owner,
        miniapp_id,
        project_id,
        operation_id,
        1,
        &source_digest,
        &dependency_digest,
        1,
        20,
        30,
    )
    .await;
    insert_succeeded_miniapp_build(
        &database,
        owner,
        miniapp_id,
        project_id,
        retired_operation_id,
        2,
        &retired_source_digest,
        &dependency_digest,
        2,
        31,
        40,
    )
    .await;
    sqlx::query(
        "INSERT INTO miniapp_release_artifacts (
            artifact_id, owner_user_id, artifact_digest, manifest_digest,
            artifact_record_json, managed_path, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, 20)",
    )
    .bind(artifact_id)
    .bind(owner)
    .bind(&artifact_digest)
    .bind(&manifest_digest)
    .bind(format!(
        r#"{{"artifact_id":"{artifact_id}","artifact_digest":"{artifact_digest}","manifest_digest":"{manifest_digest}"}}"#
    ))
    .bind(format!("artifacts/{artifact_digest}"))
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_release_artifacts (
            artifact_id, owner_user_id, artifact_digest, manifest_digest,
            artifact_record_json, managed_path, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, 21)",
    )
    .bind(retired_artifact_id)
    .bind(owner)
    .bind(&retired_artifact_digest)
    .bind(&retired_manifest_digest)
    .bind(format!(
        r#"{{"artifact_id":"{retired_artifact_id}","artifact_digest":"{retired_artifact_digest}","manifest_digest":"{retired_manifest_digest}"}}"#
    ))
    .bind(format!("artifacts/{retired_artifact_digest}"))
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_releases (
            release_id, miniapp_id, owner_user_id, artifact_id,
            artifact_digest, manifest_digest, release_digest, origin_kind,
            origin_operation_id, source_kind, project_id, source_snapshot_digest,
            dependency_lock_digest, build_profile_version, build_generation,
            release_record_json, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, 'build', ?, 'managed', ?, ?, ?,
                   ?, 1, ?, 30)",
    )
    .bind(release_id)
    .bind(miniapp_id)
    .bind(owner)
    .bind(artifact_id)
    .bind(&artifact_digest)
    .bind(&manifest_digest)
    .bind(&artifact_digest)
    .bind(operation_id)
    .bind(project_id)
    .bind(&source_digest)
    .bind(&dependency_digest)
    .bind(MINIAPP_RELEASE_PROFILE_VERSION)
    .bind(release_record_json(
        miniapp_id,
        project_id,
        artifact_id,
        release_id,
        &artifact_digest,
        &manifest_digest,
        operation_id,
        &source_digest,
        &dependency_digest,
        1,
        30,
    ))
    .execute(&database)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO miniapp_releases (
            id, release_id, miniapp_id, owner_user_id, artifact_id,
            artifact_digest, manifest_digest, release_digest, origin_kind,
            origin_operation_id, source_kind, project_id, source_snapshot_digest,
            dependency_lock_digest, build_profile_version, build_generation,
            release_record_json, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'build', ?, 'managed', ?, ?, ?,
                   ?, 2, ?, 40)",
    )
    .bind(old_sequence)
    .bind(retired_release_id)
    .bind(miniapp_id)
    .bind(owner)
    .bind(retired_artifact_id)
    .bind(&retired_artifact_digest)
    .bind(&retired_manifest_digest)
    .bind(&retired_artifact_digest)
    .bind(retired_operation_id)
    .bind(project_id)
    .bind(&retired_source_digest)
    .bind(&dependency_digest)
    .bind(MINIAPP_RELEASE_PROFILE_VERSION)
    .bind(release_record_json(
        miniapp_id,
        project_id,
        retired_artifact_id,
        retired_release_id,
        &retired_artifact_digest,
        &retired_manifest_digest,
        retired_operation_id,
        &retired_source_digest,
        &dependency_digest,
        2,
        40,
    ))
    .execute(&database)
    .await
    .unwrap();
    sqlx::query("DELETE FROM miniapp_releases WHERE release_id = ?")
        .bind(retired_release_id)
        .execute(&database)
        .await
        .unwrap();

    let sequence_before: i64 =
        sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name = 'miniapp_releases'")
            .fetch_one(&database)
            .await
            .unwrap();
    assert_eq!(sequence_before, old_sequence);

    let release_before: (
        i64,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
    ) = sqlx::query_as(
        "SELECT id, release_id, artifact_id, artifact_digest, release_digest,
                source_snapshot_digest, build_generation, release_record_json,
                created_at
         FROM miniapp_releases WHERE release_id = ?",
    )
    .bind(release_id)
    .fetch_one(&database)
    .await
    .unwrap();

    let prefix_head: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&database)
        .await
        .unwrap();
    assert_eq!(prefix_head, 77);
    database.close().await;

    let upgraded = init_database(&path)
        .await
        .expect("production startup must upgrade a valid v077 file database");
    let head: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(upgraded.pool())
        .await
        .unwrap();
    assert_eq!(head, 83);
    let quick_check: Vec<String> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_all(upgraded.pool())
        .await
        .unwrap();
    assert_eq!(quick_check, ["ok"]);

    let release_after: (
        i64,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
    ) = sqlx::query_as(
        "SELECT id, release_id, artifact_id, artifact_digest, release_digest,
                source_snapshot_digest, build_generation, release_record_json,
                created_at
         FROM miniapp_releases WHERE release_id = ?",
    )
    .bind(release_id)
    .fetch_one(upgraded.pool())
    .await
    .unwrap();
    assert_eq!(release_after, release_before);

    let catalog: (String, String, i64, String) = sqlx::query_as(
        "SELECT active_release_id, active_release_digest,
                active_release_epoch, catalog_digest
         FROM miniapp_catalog_publications
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(owner)
    .bind(miniapp_id)
    .fetch_one(upgraded.pool())
    .await
    .unwrap();
    assert_eq!(
        catalog,
        (
            release_id.to_owned(),
            artifact_digest.clone(),
            1,
            catalog_digest
        )
    );

    let sequence_after_upgrade: i64 =
        sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name = 'miniapp_releases'")
            .fetch_one(upgraded.pool())
            .await
            .unwrap();
    assert_eq!(
        sequence_after_upgrade, old_sequence,
        "table rebuild must retain the deleted-row AUTOINCREMENT high-water mark"
    );
    let helper_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table' AND name = 'miniapp_releases_v079_sequence'",
    )
    .fetch_one(upgraded.pool())
    .await
    .unwrap();
    assert_eq!(helper_table_count, 0);

    sqlx::query(
        "UPDATE miniapp_projects
         SET project_revision = 3, source_head_digest = ?,
             build_generation = 3, updated_at = 50
         WHERE owner_user_id = ? AND miniapp_id = ? AND project_id = ?",
    )
    .bind(&reused_source_digest)
    .bind(owner)
    .bind(miniapp_id)
    .bind(project_id)
    .execute(upgraded.pool())
    .await
    .unwrap();
    insert_succeeded_miniapp_build(
        upgraded.pool(),
        owner,
        miniapp_id,
        project_id,
        reused_operation_id,
        3,
        &reused_source_digest,
        &dependency_digest,
        3,
        51,
        60,
    )
    .await;
    sqlx::query(
        "INSERT INTO miniapp_releases (
            release_id, miniapp_id, owner_user_id, artifact_id,
            artifact_digest, manifest_digest, release_digest, origin_kind,
            origin_operation_id, source_kind, project_id, source_snapshot_digest,
            dependency_lock_digest, build_profile_version, build_generation,
            release_record_json, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, 'build', ?, 'managed', ?, ?, ?,
                   ?, 3, ?, 60)",
    )
    .bind(reused_release_id)
    .bind(miniapp_id)
    .bind(owner)
    .bind(artifact_id)
    .bind(&artifact_digest)
    .bind(&manifest_digest)
    .bind(&artifact_digest)
    .bind(reused_operation_id)
    .bind(project_id)
    .bind(&reused_source_digest)
    .bind(&dependency_digest)
    .bind(MINIAPP_RELEASE_PROFILE_VERSION)
    .bind(release_record_json(
        miniapp_id,
        project_id,
        artifact_id,
        reused_release_id,
        &artifact_digest,
        &manifest_digest,
        reused_operation_id,
        &reused_source_digest,
        &dependency_digest,
        3,
        60,
    ))
    .execute(upgraded.pool())
    .await
    .unwrap();

    let releases: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, release_id, source_snapshot_digest, build_generation
         FROM miniapp_releases
         WHERE owner_user_id = ? AND miniapp_id = ? AND release_digest = ?
         ORDER BY id",
    )
    .bind(owner)
    .bind(miniapp_id)
    .bind(&artifact_digest)
    .fetch_all(upgraded.pool())
    .await
    .unwrap();
    assert_eq!(releases.len(), 2);
    assert_eq!(releases[0].1, release_id);
    assert_eq!(releases[0].2, source_digest);
    assert_eq!(releases[0].3, 1);
    assert_eq!(releases[1].1, reused_release_id);
    assert_eq!(releases[1].2, reused_source_digest);
    assert_eq!(releases[1].3, 3);
    assert!(
        releases[1].0 > old_sequence,
        "next Release id must remain above the pre-upgrade sqlite_sequence"
    );

    validate_id_data_contract(upgraded.pool()).await.unwrap();
    let final_quick_check: Vec<String> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_all(upgraded.pool())
        .await
        .unwrap();
    assert_eq!(final_quick_check, ["ok"]);
    upgraded.close().await;
}
