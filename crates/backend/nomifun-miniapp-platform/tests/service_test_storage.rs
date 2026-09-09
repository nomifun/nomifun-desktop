use std::sync::Arc;

use nomifun_agent_contracts::{
    MiniAppAdditiveMigrationAction, MiniAppBridgeKvRequest, MiniAppId, MiniAppKvResponse,
    MiniAppMigration, MiniAppMigrationColumn, MiniAppMigrationId, MiniAppReleaseRef,
    StrictJsonValue, digest_bytes,
};
use nomifun_db::{
    CreateMiniAppM1Params, IMiniAppM1Repository, MiniAppM1Kind, SqliteMiniAppM1Repository,
    init_database_memory, installation_owner_id,
};
use nomifun_miniapp_platform::{
    InMemoryMiniAppManagedStorage, MiniAppCallCancellation, MiniAppDatabaseQueryResult,
    MiniAppDatabaseStatement, MiniAppServiceStoragePort, MiniAppServiceStorageRequest,
    SqliteMiniAppManagedStorage,
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

fn digest(seed: &str) -> String {
    digest_bytes(seed.as_bytes()).as_ref().to_owned()
}

async fn service_fixture() -> (
    nomifun_db::Database,
    String,
    MiniAppId,
    Arc<SqliteMiniAppManagedStorage>,
    TempDir,
) {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let miniapp_id = Uuid::now_v7().to_string();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    repository
        .create(&CreateMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: Uuid::now_v7().to_string(),
            expected_library_revision: 0,
            display_name: "Service Test storage fixture".into(),
            description: None,
            icon_asset_id: None,
            kind: MiniAppM1Kind::Service,
            materialized_catalog_digest: digest("catalog"),
            config_schema_json: r#"{"type":"object"}"#.into(),
            config_json: "{}".into(),
            created_at: 1,
        })
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let storage = Arc::new(
        SqliteMiniAppManagedStorage::new(root.path(), database.pool().clone()).unwrap(),
    );
    (
        database,
        owner,
        MiniAppId::from(miniapp_id),
        storage,
        root,
    )
}

fn create_state_migration() -> MiniAppMigration {
    MiniAppMigration::new(
        MiniAppMigrationId::from("001_create_state"),
        vec![MiniAppAdditiveMigrationAction::CreateTable {
            table_name: "state".into(),
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
                    default_literal: None,
                },
            ],
            primary_key_columns: vec!["id".into()],
        }],
    )
    .unwrap()
}

async fn kv_request(
    storage: &dyn MiniAppServiceStoragePort,
    miniapp_id: &MiniAppId,
    descriptor: &nomifun_agent_contracts::MiniAppServiceStorageDescriptor,
    request: MiniAppBridgeKvRequest,
) -> MiniAppKvResponse {
    let value = storage
        .handle_service_request(
            miniapp_id,
            descriptor,
            MiniAppServiceStorageRequest::Kv { request },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    serde_json::from_value(value.0).unwrap()
}

async fn database_query(
    storage: &dyn MiniAppServiceStoragePort,
    miniapp_id: &MiniAppId,
    descriptor: &nomifun_agent_contracts::MiniAppServiceStorageDescriptor,
    id: &str,
) -> MiniAppDatabaseQueryResult {
    let value = storage
        .handle_service_request(
            miniapp_id,
            descriptor,
            MiniAppServiceStorageRequest::DatabaseQuery {
                statement: MiniAppDatabaseStatement {
                    sql: "SELECT id, value FROM state WHERE id = ?".into(),
                    parameters: StrictJsonValue(json!([id])),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    serde_json::from_value(value.0).unwrap()
}

#[tokio::test]
async fn production_service_test_storage_is_an_exact_isolated_snapshot() {
    let (_database, owner, miniapp_id, storage, _root) = service_fixture().await;
    let production = storage
        .resolve_service_storage(&owner, &miniapp_id, true, true)
        .await
        .unwrap();
    let production_files = production.descriptor.files_dir.as_ref().unwrap();
    std::fs::write(
        std::path::Path::new(&production_files.absolute_path).join("production.txt"),
        b"production",
    )
    .unwrap();
    let written = kv_request(
        storage.as_ref(),
        &miniapp_id,
        &production.descriptor,
        MiniAppBridgeKvRequest::Set {
            key: "state".into(),
            value: StrictJsonValue(json!({"value": 7})),
        },
    )
    .await;
    assert!(matches!(written, MiniAppKvResponse::Written { revision: 1 }));

    let production_database = production.descriptor.private_database.as_ref().unwrap();
    let release = MiniAppReleaseRef {
        release_id: "release-1".into(),
        artifact_id: "artifact-1".into(),
        release_digest: digest_bytes(b"release"),
        manifest_digest: digest_bytes(b"manifest"),
    };
    storage
        .apply_additive_migrations(
            &owner,
            &miniapp_id,
            &production.descriptor,
            &production_database.migration_ledger_digest,
            &release,
            &[create_state_migration()],
            2,
        )
        .await
        .unwrap();
    let production = storage
        .resolve_service_storage(&owner, &miniapp_id, true, true)
        .await
        .unwrap();
    storage
        .handle_service_request(
            &miniapp_id,
            &production.descriptor,
            MiniAppServiceStorageRequest::DatabaseExecute {
                statement: MiniAppDatabaseStatement {
                    sql: "INSERT INTO state (id, value) VALUES (?, ?)".into(),
                    parameters: StrictJsonValue(json!(["production", 7])),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();

    let test_id = Uuid::now_v7().to_string();
    let test = storage
        .create_service_test_storage(&owner, &miniapp_id, &test_id, true, true)
        .await
        .unwrap();
    assert_eq!(test.copied_kv_digest.as_ref().len(), 64);
    assert_eq!(
        test.copied_private_database_digest
            .as_ref()
            .unwrap()
            .as_ref()
            .len(),
        64
    );
    assert_eq!(test.empty_files_dir, Some(true));
    assert_eq!(
        test.migration_ledger.as_ref().unwrap().handle_id,
        test.descriptor
            .private_database
            .as_ref()
            .unwrap()
            .handle_id
    );
    let test_files = test.descriptor.files_dir.as_ref().unwrap();
    assert!(
        std::fs::read_dir(&test_files.absolute_path)
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(
        kv_request(
            storage.as_ref(),
            &miniapp_id,
            &test.descriptor,
            MiniAppBridgeKvRequest::Get {
                key: "state".into()
            },
        )
        .await,
        MiniAppKvResponse::Value {
            value: Some(StrictJsonValue(json!({"value": 7}))),
            revision: Some(1),
        }
    );
    assert_eq!(
        database_query(storage.as_ref(), &miniapp_id, &test.descriptor, "production")
            .await
            .rows,
        vec![StrictJsonValue(json!({"id": "production", "value": 7}))]
    );
    storage
        .purge_service_test_storage(
            &Uuid::now_v7().to_string(),
            &miniapp_id,
            &test_id,
        )
        .await
        .unwrap();
    assert_eq!(
        kv_request(
            storage.as_ref(),
            &miniapp_id,
            &test.descriptor,
            MiniAppBridgeKvRequest::Get {
                key: "state".into()
            },
        )
        .await,
        MiniAppKvResponse::Value {
            value: Some(StrictJsonValue(json!({"value": 7}))),
            revision: Some(1),
        }
    );

    kv_request(
        storage.as_ref(),
        &miniapp_id,
        &test.descriptor,
        MiniAppBridgeKvRequest::Set {
            key: "state".into(),
            value: StrictJsonValue(json!({"value": 9})),
        },
    )
    .await;
    storage
        .handle_service_request(
            &miniapp_id,
            &test.descriptor,
            MiniAppServiceStorageRequest::DatabaseExecute {
                statement: MiniAppDatabaseStatement {
                    sql: "UPDATE state SET value = ? WHERE id = ?".into(),
                    parameters: StrictJsonValue(json!([9, "production"])),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    let second_test_id = Uuid::now_v7().to_string();
    let second_test = storage
        .create_service_test_storage(&owner, &miniapp_id, &second_test_id, true, true)
        .await
        .unwrap();
    assert_eq!(
        kv_request(
            storage.as_ref(),
            &miniapp_id,
            &second_test.descriptor,
            MiniAppBridgeKvRequest::Get {
                key: "state".into()
            },
        )
        .await,
        MiniAppKvResponse::Value {
            value: Some(StrictJsonValue(json!({"value": 7}))),
            revision: Some(1),
        }
    );
    assert_eq!(
        database_query(
            storage.as_ref(),
            &miniapp_id,
            &second_test.descriptor,
            "production",
        )
        .await
        .rows,
        vec![StrictJsonValue(
            json!({"id": "production", "value": 7})
        )]
    );
    assert_eq!(
        kv_request(
            storage.as_ref(),
            &miniapp_id,
            &production.descriptor,
            MiniAppBridgeKvRequest::Get {
                key: "state".into()
            },
        )
        .await,
        MiniAppKvResponse::Value {
            value: Some(StrictJsonValue(json!({"value": 7}))),
            revision: Some(1),
        }
    );
    assert_eq!(
        database_query(
            storage.as_ref(),
            &miniapp_id,
            &production.descriptor,
            "production",
        )
        .await
        .rows,
        vec![StrictJsonValue(json!({"id": "production", "value": 7}))]
    );

    let test_files_path = std::path::PathBuf::from(&test_files.absolute_path);
    storage
        .purge_service_test_storage(&owner, &miniapp_id, &test_id)
        .await
        .unwrap();
    storage
        .purge_service_test_storage(&owner, &miniapp_id, &test_id)
        .await
        .unwrap();
    assert!(!test_files_path.exists());
    assert!(
        storage
            .handle_service_request(
                &miniapp_id,
                &test.descriptor,
                MiniAppServiceStorageRequest::Kv {
                    request: MiniAppBridgeKvRequest::Get {
                        key: "state".into(),
                    },
                },
                MiniAppCallCancellation::default(),
            )
            .await
            .is_err()
    );
    assert_eq!(
        kv_request(
            storage.as_ref(),
            &miniapp_id,
            &second_test.descriptor,
            MiniAppBridgeKvRequest::Get {
                key: "state".into()
            },
        )
        .await,
        MiniAppKvResponse::Value {
            value: Some(StrictJsonValue(json!({"value": 7}))),
            revision: Some(1),
        }
    );
    storage
        .purge_service_test_storage(&owner, &miniapp_id, &second_test_id)
        .await
        .unwrap();
    assert_eq!(
        database_query(
            storage.as_ref(),
            &miniapp_id,
            &production.descriptor,
            "production",
        )
        .await
        .rows
        .len(),
        1
    );
}

#[tokio::test]
async fn production_permanent_purge_removes_unregistered_service_tests() {
    let (database, owner, miniapp_id, storage, root) = service_fixture().await;
    storage
        .resolve_service_storage(&owner, &miniapp_id, true, true)
        .await
        .unwrap();
    let test_id = Uuid::now_v7().to_string();
    let test = storage
        .create_service_test_storage(&owner, &miniapp_id, &test_id, true, true)
        .await
        .unwrap();
    let test_root = root
        .path()
        .join("service-tests")
        .join(&owner)
        .join(miniapp_id.as_ref())
        .join(&test_id);
    assert!(test_root.is_dir());
    drop(test);
    drop(storage);

    let recovered =
        SqliteMiniAppManagedStorage::new(root.path(), database.pool().clone()).unwrap();
    recovered
        .purge_service_storage(&owner, &miniapp_id)
        .await
        .unwrap();
    assert!(!test_root.exists());
    let remaining: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ?
           AND namespace LIKE 'service-test:%'",
    )
    .bind(&owner)
    .bind(miniapp_id.as_ref())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn in_memory_service_test_storage_is_isolated_and_idempotently_purged() {
    let storage = InMemoryMiniAppManagedStorage::new();
    let owner = Uuid::now_v7().to_string();
    let miniapp_id = MiniAppId::from(Uuid::now_v7().to_string());
    let production = storage
        .resolve_service_storage(&owner, &miniapp_id, true, true)
        .await
        .unwrap();
    kv_request(
        &storage,
        &miniapp_id,
        &production.descriptor,
        MiniAppBridgeKvRequest::Set {
            key: "state".into(),
            value: StrictJsonValue(json!({"value": 7})),
        },
    )
    .await;

    let test_id = Uuid::now_v7().to_string();
    let test = storage
        .create_service_test_storage(&owner, &miniapp_id, &test_id, true, true)
        .await
        .unwrap();
    kv_request(
        &storage,
        &miniapp_id,
        &test.descriptor,
        MiniAppBridgeKvRequest::Set {
            key: "state".into(),
            value: StrictJsonValue(json!({"value": 9})),
        },
    )
    .await;
    assert_eq!(
        kv_request(
            &storage,
            &miniapp_id,
            &production.descriptor,
            MiniAppBridgeKvRequest::Get {
                key: "state".into()
            },
        )
        .await,
        MiniAppKvResponse::Value {
            value: Some(StrictJsonValue(json!({"value": 7}))),
            revision: Some(1),
        }
    );
    storage
        .purge_service_test_storage(&owner, &miniapp_id, &test_id)
        .await
        .unwrap();
    storage
        .purge_service_test_storage(&owner, &miniapp_id, &test_id)
        .await
        .unwrap();
    assert!(
        storage
            .handle_service_request(
                &miniapp_id,
                &test.descriptor,
                MiniAppServiceStorageRequest::Kv {
                    request: MiniAppBridgeKvRequest::Get {
                        key: "state".into(),
                    },
                },
                MiniAppCallCancellation::default(),
            )
            .await
            .is_err()
    );
}

#[cfg(windows)]
#[tokio::test]
async fn production_service_test_purge_rejects_junctions() {
    let (_database, owner, miniapp_id, storage, root) = service_fixture().await;
    let test_id = Uuid::now_v7().to_string();
    let test_parent = root
        .path()
        .join("service-tests")
        .join(&owner)
        .join(miniapp_id.as_ref());
    std::fs::create_dir_all(&test_parent).unwrap();
    let outside = root.path().join("outside-service-test");
    std::fs::create_dir_all(&outside).unwrap();
    let marker = outside.join("keep.txt");
    std::fs::write(&marker, b"keep").unwrap();
    let test_root = test_parent.join(&test_id);
    junction::create(&outside, &test_root).unwrap();

    assert!(
        storage
            .purge_service_test_storage(&owner, &miniapp_id, &test_id)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
    junction::delete(&test_root).unwrap();
}
