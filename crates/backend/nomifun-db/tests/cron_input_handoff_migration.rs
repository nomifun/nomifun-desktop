//! Upgrade the supported prefix without changing task data or sequence history.
use std::borrow::Cow;

use nomifun_agent_contracts::{
    AGENT_STORE_DATA_GENERATION, AGENT_STORE_MIGRATION_HEAD, AGENT_STORE_PROJECTION_SCHEMA_VERSION,
    agent_store_schema_manifest_payload, digest_payload, official_preset_seed_manifest_payload,
};
use nomifun_db::{MigrationLineage, init_database, inspect_migration_lineage};
use sqlx::{SqlitePool, migrate::Migrator, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

static MIGRATOR: Migrator = sqlx::migrate!();

async fn rows(pool: &SqlitePool, table: &str) -> Vec<String> {
    let columns: Vec<String> = sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .fetch_all(pool).await.unwrap();
    let fields = columns.iter().map(|name| format!("\"{name}\"")).collect::<Vec<_>>().join(",");
    sqlx::query_scalar(&format!("SELECT json_array({fields}) FROM {table} ORDER BY id"))
        .fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn forward_009_preserves_cron_rows_indexes_sequences_and_supported_lineage_on_restart() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("cron-input-handoff.sqlite");
    let previous = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(&path).create_if_missing(true).foreign_keys(false),
    ).await.unwrap();
    Migrator { migrations: Cow::Owned(MIGRATOR.iter().take(8).cloned().collect()), ..Migrator::DEFAULT }
        .run(&previous).await.unwrap();
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
        .bind(digest_payload(&official_preset_seed_manifest_payload()).unwrap().as_ref())
        .bind(digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref())
        .bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)).execute(&previous).await.unwrap();
    let job = "0190f5fe-7c00-7a00-8000-000000000001";
    let run = "0190f5fe-7c00-7a00-8000-000000000002";
    let owner = "0190f5fe-7c00-7a00-8000-000000000003";
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES(?,'cron-upgrade-owner','fixture-hash',100,100)")
        .bind(owner).execute(&previous).await.unwrap();
    sqlx::query("INSERT INTO cron_jobs(id,cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,agent_type,created_by,created_at,updated_at,last_status,last_error,run_count,retry_count) VALUES(7,?,?,'Original schedule','every','60000','Preserve this task','nomifun.nomi','user',100,200,'error','Original diagnostic',4,1)")
        .bind(job).bind(owner).execute(&previous).await.unwrap();
    sqlx::query("INSERT INTO cron_job_runs(id,cron_job_run_id,cron_job_id,executed_at_ms,status,created_at_ms) VALUES(9,?,?,100,'error',100)")
        .bind(run).bind(job).execute(&previous).await.unwrap();
    sqlx::query("INSERT INTO cron_run_reservations(id,cron_job_run_id,cron_job_id,trigger_kind,operation_key,request_fingerprint,status,created_at_ms,updated_at_ms,settled_at_ms,job_projection_state,job_projected_at_ms) VALUES(11,?,?,'run_now','original-operation','original-fingerprint','error',100,200,200,'applied',200)")
        .bind(run).bind(job).execute(&previous).await.unwrap();
    // Deleted high IDs must not be reused by the table replacement.
    for table in ["cron_jobs", "cron_job_runs", "cron_run_reservations"] {
        sqlx::query("UPDATE sqlite_sequence SET seq=900 WHERE name=?").bind(table).execute(&previous).await.unwrap();
    }
    sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES('theme','\"dark\"',123)")
        .execute(&previous).await.unwrap();
    let job_rows = rows(&previous, "cron_jobs").await;
    let run_rows = rows(&previous, "cron_job_runs").await;
    let reservation_rows = rows(&previous, "cron_run_reservations").await;
    let index_query = "SELECT name,sql FROM sqlite_schema WHERE type='index' AND sql IS NOT NULL AND tbl_name IN ('cron_jobs','cron_job_runs','cron_run_reservations') ORDER BY name";
    let indexes: Vec<(String, String)> = sqlx::query_as(index_query).fetch_all(&previous).await.unwrap();
    let metadata: Vec<(i64, i64, String)> = sqlx::query_as("SELECT data_generation,migration_head,canonical_schema_manifest_digest FROM schema_metadata")
        .fetch_all(&previous).await.unwrap();
    let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&previous).await.unwrap();
    assert_eq!(inspect_migration_lineage(&previous).await.unwrap(), MigrationLineage::KnownPrefix);
    previous.close().await;
    for _ in 0..2 {
        let current = init_database(&path).await.unwrap();
        let pool = current.pool();
        assert_eq!(inspect_migration_lineage(pool).await.unwrap(), MigrationLineage::Current);
        assert_eq!(rows(pool, "cron_jobs").await, job_rows);
        assert_eq!(rows(pool, "cron_job_runs").await, run_rows);
        assert_eq!(rows(pool, "cron_run_reservations").await, reservation_rows);
        assert_eq!(sqlx::query_as::<_, (String, String)>(index_query).fetch_all(pool).await.unwrap(), indexes);
        assert_eq!(sqlx::query_as::<_, (i64, i64, String)>("SELECT data_generation,migration_head,canonical_schema_manifest_digest FROM schema_metadata")
            .fetch_all(pool).await.unwrap(), metadata);
        let retained: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations WHERE version<=8 ORDER BY version")
            .fetch_all(pool).await.unwrap();
        assert_eq!(retained, receipts);
        let sequences: Vec<i64> = sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name IN ('cron_jobs','cron_job_runs','cron_run_reservations')")
            .fetch_all(pool).await.unwrap();
        assert_eq!(sequences, vec![900; 3]);
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT value FROM client_preferences WHERE key='theme'").fetch_one(pool).await.unwrap(), "\"dark\"");
        let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check").fetch_one(pool).await.unwrap();
        assert_eq!(violations, 0);
        for status in ["input_required", "cancelled"] {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("UPDATE cron_jobs SET last_status=?,last_error=NULL").bind(status).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE cron_job_runs SET status=?").bind(status).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE cron_run_reservations SET status=?,result_error=NULL").bind(status).execute(&mut *tx).await.unwrap();
            assert_eq!(sqlx::query_scalar::<_, String>("SELECT last_status FROM cron_jobs").fetch_one(&mut *tx).await.unwrap(), status);
            tx.rollback().await.unwrap();
        }
        current.close().await;
    }
}
