//! Current-generation additive office migration: preserve canonical bytes,
//! accept only the exact non-Agent predecessor, and never obtain rebuild power.
use std::path::Path;
use nomifun_agent_contracts::{agent_store_schema_manifest_payload, digest_payload, official_preset_seed_manifest_payload};
use sha2::{Digest,Sha256};
use sqlx::{SqlitePool, sqlite::{SqliteConnectOptions,SqlitePoolOptions}};

const BEFORE_OFFICE:&str="ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9";
const OWNER:&str="0190f5fe-7c00-7a00-8000-000000000214";
const SESSION:&str="0190f5fe-7c00-7a00-8000-000000000215";

async fn old_prefix(path:&Path,digest:&str)->SqlitePool{
    let pool=SqlitePoolOptions::new().max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(false))
        .await.unwrap();
    sqlx::raw_sql("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY,description TEXT NOT NULL,installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,success BOOLEAN NOT NULL,checksum BLOB NOT NULL,execution_time BIGINT NOT NULL)")
        .execute(&pool).await.unwrap();
    let migrator=sqlx::migrate!("./migrations");
    for migration in migrator.iter().filter(|m|m.version<16){
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (?,?,1,?,0)")
            .bind(migration.version).bind(&migration.description).bind(migration.checksum.as_ref())
            .execute(&pool).await.unwrap();
    }
    let seed=digest_payload(&official_preset_seed_manifest_payload()).unwrap();
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES ('canonical',7,'main-sqlite-agent-store',1,?,?,1)")
        .bind(seed.as_ref()).bind(digest).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES (?,'office-upgrade-owner','',1,1)")
        .bind(OWNER).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO installation_identity(singleton_key,owner_user_id) VALUES ('installation',?)")
        .bind(OWNER).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',1,1)")
        .bind(SESSION).execute(&pool).await.unwrap();
    let bytes="保留 canonical 内容\nincluding\u{0}exact bytes".as_bytes();
    sqlx::query("INSERT INTO agent_payloads(payload_id,session_id,media_type,byte_len,digest,body) VALUES ('office-upgrade-payload',?,'text/plain',?,?,?)")
        .bind(SESSION).bind(bytes.len() as i64).bind(format!("{:x}",Sha256::digest(bytes))).bind(bytes)
        .execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn exact_current_generation_upgrade_adds_office_tables_without_rebuilding_agent_facts(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("known-prefix.db");
    let prefix=old_prefix(&path,BEFORE_OFFICE).await;
    assert_eq!(nomifun_db::inspect_migration_lineage(&prefix).await.unwrap(),nomifun_db::MigrationLineage::KnownPrefix);
    let before:Vec<u8>=sqlx::query_scalar("SELECT body FROM agent_payloads WHERE payload_id='office-upgrade-payload'")
        .fetch_one(&prefix).await.unwrap();
    let receipts:Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version").fetch_all(&prefix).await.unwrap();
    prefix.close().await;
    let upgraded=nomifun_db::init_database(&path).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT owner_user_id FROM installation_identity").fetch_one(upgraded.pool()).await.unwrap(),OWNER);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_sessions WHERE agent_session_id=?").bind(SESSION).fetch_one(upgraded.pool()).await.unwrap(),1);
    let after:Vec<u8>=sqlx::query_scalar("SELECT body FROM agent_payloads WHERE payload_id='office-upgrade-payload'").fetch_one(upgraded.pool()).await.unwrap();
    assert_eq!(before,after);
    let previous_receipts:Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations WHERE version<16 ORDER BY version")
        .fetch_all(upgraded.pool()).await.unwrap();
    assert_eq!(receipts,previous_receipts,"published migration checksums stay exact");
    let current_digest:String=sqlx::query_scalar("SELECT canonical_schema_manifest_digest FROM schema_metadata").fetch_one(upgraded.pool()).await.unwrap();
    assert_eq!(current_digest,digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref());
    nomifun_db::validate_id_schema_contract(upgraded.pool()).await.unwrap();
    nomifun_db::validate_id_data_contract(upgraded.pool()).await.unwrap();
    assert!(!nomifun_db::requires_agent_store_clean_cut(upgraded.pool()).await.unwrap());
    upgraded.close().await;
}

#[tokio::test]
async fn unknown_manifest_is_rejected_without_reset_or_partial_office_installation(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("unknown-prefix.db");
    let prefix=old_prefix(&path,&"a".repeat(64)).await;
    prefix.close().await;
    assert!(nomifun_db::init_database(&path).await.is_err());
    let pool=SqlitePoolOptions::new().max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(false)).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_sessions WHERE agent_session_id=?").bind(SESSION).fetch_one(&pool).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='office_operations'").fetch_one(&pool).await.unwrap(),0);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=16").fetch_one(&pool).await.unwrap(),0);
    pool.close().await;
}
