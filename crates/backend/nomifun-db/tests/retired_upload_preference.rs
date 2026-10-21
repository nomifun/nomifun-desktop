use std::{borrow::Cow, path::Path};

use nomifun_agent_contracts::{
    AGENT_STORE_DATA_GENERATION, AGENT_STORE_MIGRATION_HEAD, AGENT_STORE_PROJECTION_SCHEMA_VERSION,
    agent_store_schema_manifest_payload, digest_payload, official_preset_seed_manifest_payload,
};
use nomifun_db::{MigrationLineage, init_database, inspect_migration_lineage};
use sqlx::{SqlitePool, migrate::Migrator, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

static MIGRATOR: Migrator = sqlx::migrate!();

async fn previous_database(path: &Path) -> SqlitePool {
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(false),
    ).await.unwrap();
    let previous = Migrator { migrations: Cow::Owned(MIGRATOR.iter().take(5).cloned().collect()), ..Migrator::DEFAULT };
    previous.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
        .bind(digest_payload(&official_preset_seed_manifest_payload()).unwrap().as_ref())
        .bind(digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref())
        .bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)).execute(&pool).await.unwrap();
    sqlx::query("UPDATE system_settings SET language='zh-CN',notification_enabled=0,cron_notification_enabled=1,command_queue_enabled=1,save_upload_to_workspace=1,updated_at=99")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES ('upload.saveToWorkspace','true',1),('saveUploadToWorkspace','false',2),('system.autoPreviewOfficeFiles','false',3),('theme','\"dark\"',4)")
        .execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn forward_006_retires_only_unused_upload_configuration_on_upgrade_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("upload-preference.sqlite");
    let old = previous_database(&path).await;
    assert_eq!(inspect_migration_lineage(&old).await.unwrap(), MigrationLineage::KnownPrefix);
    let schema: Vec<(String, String)> = sqlx::query_as("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'system_settings' ORDER BY name")
        .fetch_all(&old).await.unwrap();
    let metadata: (i64, i64, String) = sqlx::query_as("SELECT data_generation,migration_head,canonical_schema_manifest_digest FROM schema_metadata")
        .fetch_one(&old).await.unwrap();
    let settings: (i64, String, String, bool, bool, i64) = sqlx::query_as("SELECT id,singleton_key,language,notification_enabled,cron_notification_enabled,updated_at FROM system_settings")
        .fetch_one(&old).await.unwrap();
    let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&old).await.unwrap();
    // Prove this migration's schema scope at its own boundary. Later canonical
    // and product migrations legitimately change other tables and are covered
    // by their own upgrade tests.
    let through_retirement = Migrator {
        migrations: Cow::Owned(MIGRATOR.iter().take_while(|migration| migration.version <= 6).cloned().collect()),
        ..Migrator::DEFAULT
    };
    through_retirement.run(&old).await.unwrap();
    assert_eq!(sqlx::query_as::<_, (String, String)>("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'system_settings' ORDER BY name")
        .fetch_all(&old).await.unwrap(), schema);
    assert_eq!(sqlx::query_as::<_, (i64, i64, String)>("SELECT data_generation,migration_head,canonical_schema_manifest_digest FROM schema_metadata")
        .fetch_one(&old).await.unwrap(), metadata);
    old.close().await;

    for _ in 0..2 {
        let current = init_database(&path).await.unwrap();
        assert_eq!(inspect_migration_lineage(current.pool()).await.unwrap(), MigrationLineage::Current);
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('system_settings')")
            .fetch_all(current.pool()).await.unwrap();
        assert!(!columns.iter().any(|column| column == "save_upload_to_workspace"));
        assert_eq!(sqlx::query_as::<_, (i64, String, String, bool, bool, i64)>("SELECT id,singleton_key,language,notification_enabled,cron_notification_enabled,updated_at FROM system_settings")
            .fetch_one(current.pool()).await.unwrap(), settings);
        let preferences: Vec<(String, String, i64)> = sqlx::query_as("SELECT key,value,updated_at FROM client_preferences ORDER BY key")
            .fetch_all(current.pool()).await.unwrap();
        assert_eq!(preferences, vec![
            ("system.autoPreviewOfficeFiles".into(), "false".into(), 3),
            ("theme".into(), "\"dark\"".into(), 4),
        ]);
        assert_eq!(sqlx::query_as::<_, (i64, i64, String)>("SELECT data_generation,migration_head,canonical_schema_manifest_digest FROM schema_metadata")
            .fetch_one(current.pool()).await.unwrap(), metadata);
        let current_receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(current.pool()).await.unwrap();
        assert_eq!(&current_receipts[..5], receipts.as_slice());
        assert_eq!(current_receipts, MIGRATOR.iter().map(|migration| (migration.version, migration.checksum.to_vec())).collect::<Vec<_>>());
        current.close().await;
    }
}

#[tokio::test]
async fn unknown_lineage_does_not_authorize_upload_preference_removal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unknown.sqlite");
    let old = previous_database(&path).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=5")
        .execute(&old).await.unwrap();
    assert!(init_database(&path).await.is_err());
    assert!(sqlx::query_scalar::<_, bool>("SELECT save_upload_to_workspace FROM system_settings")
        .fetch_one(&old).await.unwrap());
    let preferences: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key IN ('upload.saveToWorkspace','saveUploadToWorkspace')")
        .fetch_one(&old).await.unwrap();
    assert_eq!(preferences, 2);
    old.close().await;
}
