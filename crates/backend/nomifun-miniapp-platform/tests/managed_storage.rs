use std::sync::Arc;

use nomifun_agent_contracts::{
    MiniAppAdditiveMigrationAction, MiniAppBridgeKvRequest, MiniAppKvResponse, MiniAppMigration,
    MiniAppMigrationColumn, MiniAppMigrationId, MiniAppReleaseRef, StrictJsonValue,
    digest_bytes,
};
use nomifun_db::{
    CreateMiniAppM1Params, IMiniAppM1Repository, MiniAppM1Kind, SqliteMiniAppM1Repository,
    init_database_memory, installation_owner_id,
};
use nomifun_miniapp_platform::{
    MiniAppCallCancellation, MiniAppDatabaseQueryResult, MiniAppDatabaseStatement,
    MiniAppServiceStoragePort, MiniAppServiceStorageRequest, SqliteMiniAppManagedStorage,
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
    String,
    Arc<SqliteMiniAppManagedStorage>,
    TempDir,
) {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let miniapp_id = Uuid::now_v7().to_string();
    let project_id = Uuid::now_v7().to_string();
    let repository = SqliteMiniAppM1Repository::new(database.pool().clone());
    repository
        .create(&CreateMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id,
            expected_library_revision: 0,
            display_name: "Managed storage fixture".into(),
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
        SqliteMiniAppManagedStorage::new(root.path().to_path_buf(), database.pool().clone())
            .unwrap(),
    );
    (database, owner, miniapp_id, storage, root)
}

#[tokio::test]
async fn production_storage_is_owner_scoped_and_persists_private_db() {
    let (_database, owner, miniapp_id, storage, root) = service_fixture().await;
    let miniapp = nomifun_agent_contracts::MiniAppId::from(miniapp_id.clone());
    let resolved = storage
        .resolve_service_storage(&owner, &miniapp, true, true)
        .await
        .unwrap();
    let files = resolved.descriptor.files_dir.as_ref().unwrap();
    assert!(std::path::Path::new(&files.absolute_path).is_dir());
    assert!(resolved.descriptor.private_database.is_some());
    assert!(!root
        .path()
        .join("databases")
        .join(&owner)
        .join(format!("{miniapp_id}.sqlite"))
        .to_string_lossy()
        .contains("absolute_path"));

    let kv = storage
        .handle_service_request(
            &miniapp,
            &resolved.descriptor,
            MiniAppServiceStorageRequest::Kv {
                request: MiniAppBridgeKvRequest::Set {
                    key: "state".into(),
                    value: StrictJsonValue(json!({"value": 7})),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    let written: MiniAppKvResponse = serde_json::from_value(kv.0).unwrap();
    assert!(matches!(written, MiniAppKvResponse::Written { revision: 1 }));

    let migration = MiniAppMigration::new(
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
                    default_literal: Some("0".into()),
                },
            ],
            primary_key_columns: vec!["id".into()],
        }],
    )
    .unwrap();
    let database = resolved.descriptor.private_database.as_ref().unwrap();
    let release = MiniAppReleaseRef {
        release_id: "release-1".into(),
        artifact_id: "artifact-1".into(),
        release_digest: digest_bytes(b"release"),
        manifest_digest: digest_bytes(b"manifest"),
    };
    let ledger = storage
        .apply_additive_migrations(
            &owner,
            &miniapp,
            &resolved.descriptor,
            &database.migration_ledger_digest,
            &release,
            std::slice::from_ref(&migration),
            2,
        )
        .await
        .unwrap();
    assert_eq!(ledger.entries.len(), 1);
    assert_eq!(ledger.schema_epoch, 2);
    let resolved_after_migration = storage
        .resolve_service_storage(&owner, &miniapp, true, true)
        .await
        .unwrap();

    let inserted = storage
        .handle_service_request(
            &miniapp,
            &resolved_after_migration.descriptor,
            MiniAppServiceStorageRequest::DatabaseExecute {
                statement: MiniAppDatabaseStatement {
                    sql: "INSERT INTO state (id, value) VALUES (?, ?)".into(),
                    parameters: StrictJsonValue(json!(["one", 7])),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    let inserted: nomifun_miniapp_platform::MiniAppDatabaseExecuteResult =
        serde_json::from_value(inserted.0).unwrap();
    assert_eq!(inserted.affected_rows, 1);

    let queried = storage
        .handle_service_request(
            &miniapp,
            &resolved_after_migration.descriptor,
            MiniAppServiceStorageRequest::DatabaseQuery {
                statement: MiniAppDatabaseStatement {
                    sql: "SELECT id, value FROM state WHERE id = ?".into(),
                    parameters: StrictJsonValue(json!(["one"])),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    let queried: MiniAppDatabaseQueryResult = serde_json::from_value(queried.0).unwrap();
    assert_eq!(queried.rows, vec![StrictJsonValue(json!({"id": "one", "value": 7}))]);

    let forbidden = storage
        .handle_service_request(
            &miniapp,
            &resolved_after_migration.descriptor,
            MiniAppServiceStorageRequest::DatabaseQuery {
                statement: MiniAppDatabaseStatement {
                    sql: "PRAGMA user_version".into(),
                    parameters: StrictJsonValue(json!([])),
                },
            },
            MiniAppCallCancellation::default(),
        )
        .await;
    assert!(forbidden.is_err());

    let restarted = SqliteMiniAppManagedStorage::new(
        root.path().to_path_buf(),
        _database.pool().clone(),
    )
    .unwrap();
    let resolved_again = restarted
        .resolve_service_storage(&owner, &miniapp, true, true)
        .await
        .unwrap();
    assert_eq!(
        resolved_again
            .descriptor
            .private_database
            .unwrap()
            .migration_ledger_digest,
        ledger.ledger_digest
    );
}
