use std::{borrow::Cow, path::Path};
use nomifun_agent_contracts::{
    AGENT_STORE_DATA_GENERATION, AGENT_STORE_MIGRATION_HEAD, AGENT_STORE_PROJECTION_SCHEMA_VERSION,
    agent_store_schema_manifest_payload, digest_payload, official_preset_seed_manifest_payload,
};
use sqlx::{SqlitePool, migrate::Migrator, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

static MIGRATOR: Migrator = sqlx::migrate!();

async fn previous_database(path: &Path) -> SqlitePool {
    previous_database_at(path, 9).await
}

async fn previous_database_at(path: &Path, migration_count: usize) -> SqlitePool {
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(false),
    ).await.unwrap();
    Migrator { migrations: Cow::Owned(MIGRATOR.iter().take(migration_count).cloned().collect()), ..Migrator::DEFAULT }
        .run(&pool).await.unwrap();
    let mut old_manifest = agent_store_schema_manifest_payload();
    old_manifest.tables.retain(|table| !matches!(table.table_name.as_str(), "mcp_oauth_clients" | "mcp_oauth_grants"));
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
        .bind(digest_payload(&official_preset_seed_manifest_payload()).unwrap().as_ref())
        .bind(digest_payload(&old_manifest).unwrap().as_ref())
        .bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO mcp_servers(mcp_server_id,name,transport_type,transport_config,tools,original_json,created_at,updated_at) VALUES('0190f5fe-7c00-7a00-8000-000000000051','Preserve MCP','http','{\"url\":\"https://example.com/mcp\"}','[]','original',11,12)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO oauth_tokens(server_url,access_token,refresh_token,token_type,created_at,updated_at) VALUES('https://example.com/mcp','legacy-access','legacy-refresh','bearer',1,1)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES('oauth-upgrade-preserve','preserved preference',7)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES('0190f5fe-7c00-7a00-8000-000000000052','{}','live','Preserve Session',0,0,'{}',1,10)")
        .execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn forward_010_retires_only_unbound_credentials_and_preserves_configuration_and_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("oauth.sqlite");
    let old = previous_database(&path).await;
    let server: String = sqlx::query_scalar("SELECT json_array(mcp_server_id,name,enabled,transport_type,transport_config,tools,original_json,created_at,updated_at) FROM mcp_servers")
        .fetch_one(&old).await.unwrap();
    let old_receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&old).await.unwrap();
    old.close().await;
    for _ in 0..2 {
        let db = nomifun_db::init_database(&path).await.unwrap();
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT json_array(mcp_server_id,name,enabled,transport_type,transport_config,tools,original_json,created_at,updated_at) FROM mcp_servers")
            .fetch_one(db.pool()).await.unwrap(), server);
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT value FROM client_preferences WHERE key='oauth-upgrade-preserve'")
            .fetch_one(db.pool()).await.unwrap(), "preserved preference");
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT title FROM agent_sessions WHERE agent_session_id='0190f5fe-7c00-7a00-8000-000000000052'")
            .fetch_one(db.pool()).await.unwrap(), "Preserve Session");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE name='oauth_tokens'")
            .fetch_one(db.pool()).await.unwrap(), 0);
        for table in ["mcp_oauth_clients", "mcp_oauth_grants"] {
            assert_eq!(sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(db.pool()).await.unwrap(), 0);
        }
        let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(db.pool()).await.unwrap();
        assert_eq!(&receipts[..9], old_receipts.as_slice());
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT canonical_schema_manifest_digest FROM schema_metadata")
            .fetch_one(db.pool()).await.unwrap(), digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref());
        db.close().await;
    }
}

#[tokio::test]
async fn invalid_metadata_refuses_retirement_without_deleting_old_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("damaged-metadata.sqlite");
    let previous = previous_database(&path).await;
    sqlx::query("UPDATE schema_metadata SET canonical_schema_manifest_digest=?")
        .bind("0".repeat(64)).execute(&previous).await.unwrap();
    assert!(nomifun_db::init_database(&path).await.is_err());
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT access_token FROM oauth_tokens")
        .fetch_one(&previous).await.unwrap(), "legacy-access");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=10")
        .fetch_one(&previous).await.unwrap(), 0);
}

#[tokio::test]
async fn known_pre_plugin_retirement_prefix_accepts_the_exact_previous_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("before-plugin-retirement.sqlite");
    let previous = previous_database_at(&path, 3).await;
    previous.close().await;
    let current = nomifun_db::init_database(&path).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT title FROM agent_sessions WHERE agent_session_id='0190f5fe-7c00-7a00-8000-000000000052'")
        .fetch_one(current.pool()).await.unwrap(), "Preserve Session");
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT canonical_schema_manifest_digest FROM schema_metadata")
        .fetch_one(current.pool()).await.unwrap(), digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref());
}
