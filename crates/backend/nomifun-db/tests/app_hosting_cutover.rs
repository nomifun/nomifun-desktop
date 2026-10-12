use std::{borrow::Cow, fs};
use sqlx::{Connection, sqlite::{SqliteConnectOptions, SqliteJournalMode}, migrate::Migrator, SqliteConnection};
use nomifun_db::init_database;

async fn previous_database(path: &std::path::Path) -> SqliteConnection {
    let opts=SqliteConnectOptions::new().filename(path).create_if_missing(true).journal_mode(SqliteJournalMode::Wal);
    let mut connection=SqliteConnection::connect_with(&opts).await.unwrap();
    let all=sqlx::migrate!("./migrations");
    let previous=Migrator { migrations: Cow::Owned(all.iter().take(3).cloned().collect()), ..Migrator::DEFAULT };
    sqlx::query("PRAGMA foreign_keys=OFF").execute(&mut connection).await.unwrap();
    previous.run(&mut connection).await.unwrap();
    sqlx::query("PRAGMA foreign_keys=ON").execute(&mut connection).await.unwrap();
    use nomifun_agent_contracts::{AGENT_STORE_DATA_GENERATION, AGENT_STORE_MIGRATION_HEAD,
        AGENT_STORE_PROJECTION_SCHEMA_VERSION, agent_store_schema_manifest_payload,
        official_preset_seed_manifest_payload, digest_payload};
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES ('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
        .bind(digest_payload(&official_preset_seed_manifest_payload()).unwrap().as_ref())
        .bind(digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref())
        .bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)).execute(&mut connection).await.unwrap();
    connection
}

#[tokio::test]
async fn forward_cutover_preserves_committed_wal_works_and_canonical_rows() {
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("nomifun-backend.db");
    let mut old=previous_database(&path).await;
    let session=uuid::Uuid::now_v7().to_string();
    let event=uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',2,1)")
        .bind(&session).execute(&mut old).await.unwrap();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES (?,1,?,'test','opening','session/opening',1,?,'{\"kept\":true}')")
        .bind(&session).bind(&event).bind(&event).execute(&mut old).await.unwrap();
    let receipts: Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version").fetch_all(&mut old).await.unwrap();
    old.close().await.unwrap();
    let work=temp.path().join("plugin-data").join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("source.js"),"ordinary source preserved").unwrap();
    let opts=SqliteConnectOptions::new().filename(work.join("data.sqlite")).create_if_missing(true).journal_mode(SqliteJournalMode::Wal);
    let mut business=SqliteConnection::connect_with(&opts).await.unwrap();
    sqlx::raw_sql("PRAGMA wal_autocheckpoint=0; CREATE TABLE entries(body TEXT); INSERT INTO entries VALUES ('committed in WAL');")
        .execute(&mut business).await.unwrap();
    assert!(work.join("data.sqlite-wal").exists());
    let database=init_database(&path).await.unwrap();
    let no_plugins: i64=sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('plugins','plugin_artifacts','plugin_drafts','plugin_mutations','plugin_grants','plugin_library_state','plugin_credential_bindings')")
        .fetch_one(database.pool()).await.unwrap();
    assert_eq!(no_plugins,0);
    let retained: String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?").bind(&event).fetch_one(database.pool()).await.unwrap();
    assert_eq!(retained,"{\"kept\":true}");
    let current: Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version").fetch_all(database.pool()).await.unwrap();
    assert_eq!(&current[..3],receipts.as_slice());
    assert_eq!(current.len(),4);
    let directory=fs::read_dir(temp.path().join("preserved-plugin-works")).unwrap().next().unwrap().unwrap().path();
    assert_eq!(fs::read_to_string(directory.join("plugin-data/work/source.js")).unwrap(),"ordinary source preserved");
    assert!(directory.join("registration.json").exists());
    let mut saved=SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(directory.join("plugin-data/work/data.sqlite")).read_only(true)).await.unwrap();
    let value: String=sqlx::query_scalar("SELECT body FROM entries").fetch_one(&mut saved).await.unwrap();
    assert_eq!(value,"committed in WAL");
    assert!(work.join("source.js").exists());
    saved.close().await.unwrap();
    business.close().await.unwrap();
    database.close().await;
}

#[tokio::test]
async fn failed_work_protection_does_not_apply_retirement() {
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("nomifun-backend.db");
    previous_database(&path).await.close().await.unwrap();
    // A regular file at a required directory is an actual filesystem failure.
    fs::write(temp.path().join("preserved-plugin-works"),"blocked").unwrap();
    assert!(init_database(&path).await.is_err());
    let mut original=SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path).read_only(true)).await.unwrap();
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations").fetch_one(&mut original).await.unwrap();
    assert_eq!(count,3);
    let table:i64=sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name='plugins'").fetch_one(&mut original).await.unwrap();
    assert_eq!(table,1);
    original.close().await.unwrap();
}

#[tokio::test]
async fn incomplete_or_mismatched_metadata_cannot_retire_works_with_valid_receipts() {
    for missing in [false, true] {
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("nomifun-backend.db");
        let mut original=previous_database(&path).await;
        if missing {
            sqlx::query("DELETE FROM schema_metadata").execute(&mut original).await.unwrap();
        } else {
            sqlx::query("UPDATE schema_metadata SET canonical_schema_manifest_digest=?")
                .bind("f".repeat(64)).execute(&mut original).await.unwrap();
        }
        original.close().await.unwrap();
        assert!(init_database(&path).await.is_err());
        let mut original=SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path).read_only(true)).await.unwrap();
        let receipt_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations").fetch_one(&mut original).await.unwrap();
        assert_eq!(receipt_count,3);
        let plugin_table:i64=sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name='plugins'").fetch_one(&mut original).await.unwrap();
        assert_eq!(plugin_table,1);
        assert!(!temp.path().join("preserved-plugin-works").exists());
        original.close().await.unwrap();
    }
}
