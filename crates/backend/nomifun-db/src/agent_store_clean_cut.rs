//! One-time Agent-only generation cutover. No retired Agent data is decoded.
use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{AGENT_STORE_BASELINE_SQL, AGENT_STORE_DATA_GENERATION,
    AGENT_STORE_MIGRATION_HEAD, AGENT_STORE_PROJECTION_SCHEMA_VERSION,
    agent_store_schema_manifest_payload, digest_payload, official_preset_seed_manifest_payload};
use sqlx::{Connection, Row, SqliteConnection, SqlitePool, Transaction};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use crate::DbError;
use crate::agent_store_reset::{RESET_ORDER, reset_agent_data_in_transaction};

// Published generation-6 receipts identify the only retired data generation
// eligible for a clean cut. They authorize deletion, never data conversion.
const RETIRED_CHECKSUMS: &[&str] = &[
    "637c797311953caf73471175eccf4ce37bcaef3d2069566f7f428cf62a24e3cb502b41c05ec0523202d80f64d341d4dc",
    "8fc12d90cdec8065377cb79415e71c14f66fcbc281862727d93584389b91a7496c771fc028308f4db4712c93c7ae8ead",
    "b49bf6b4254339a2156e1ecbd2d32e3e1ffe3bd0abd35d60f9cb347dc3805c5d6eb0630d7e41a61e191211a3d7aec5f9",
    "f970740bd1875e1499f224060d0cc3db804aeebe9b0620004ba6157588bb75cf2c51775fe0da475aa746d8cfbbf66737",
    "c41b01a325f085f3d1b3643eac930beba46001b9bb23ba5ad6c68bacb49ef31fa302564bcb6257ad0b264004f2d18cb5",
    "124ce3c5e6eb08d8ffe5b3f16507a6212b87e9c02a90823641eb9a63ad0ef66cb3a78c8acdd22399d411ff6c02ce7120",
    "af1e0ce565ce7dd753cd9c130da7fc5c40fb2778de03c4aa7ce6db606f625bc4dae86972fe834ef484a7e992e234e702",
    "0c388bb00c38c60c617e010f3fce55e3f0da48fb31c3c27d2e8d5aa5dbc0dbee872c6e5e58e73f964abba0cc4a6080da",
    "cd5c91e30d1bd9a7f4d22a3090d3b0ab3ad8e7e84b7543545ddd04a9b22e41c897aac1c126b1be3583391aab2e738c13",
    "10430c37db28a8677dd149213aee58fb670d5eefe2a2ef7089baf6e2ed92b47883688f635e2038060ff7f0c7d8ebb402",
    "4de42e9015e521d3a2100a40c849e5791d1d4a485a9be175c844f94393d01c6e67aba167eb2de3f57b1348074e4bca35",
];

fn exact_retired_receipts(rows: &[SqliteRow], generation: i64) -> bool {
    generation == 6 && rows.len() == RETIRED_CHECKSUMS.len()
        && rows.iter().enumerate().all(|(index, row)| {
            row.try_get::<i64, _>("version").ok() == Some(index as i64 + 1)
                && row.try_get::<bool, _>("success").ok() == Some(true)
                && row.try_get::<Vec<u8>, _>("checksum").ok()
                    .is_some_and(|checksum| hex::encode(checksum) == RETIRED_CHECKSUMS[index])
        })
}

/// Recognize only the complete published generation-6 lineage. Unknown,
/// partial and hand-edited receipts are never an automatic-reset authority.
pub async fn requires_agent_store_clean_cut(pool: &SqlitePool) -> Result<bool, DbError> {
    let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN ('_sqlx_migrations','schema_metadata')")
        .fetch_one(pool).await?;
    if tables != 2 { return Ok(false); }
    let generation: Option<i64> = sqlx::query_scalar("SELECT data_generation FROM schema_metadata WHERE singleton_key='canonical'")
        .fetch_optional(pool).await?;
    let rows = sqlx::query("SELECT version,success,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool).await?;
    Ok(exact_retired_receipts(&rows, generation.unwrap_or(0)))
}

pub(crate) async fn ensure_schema_metadata(conn: &mut SqliteConnection) -> Result<(), DbError> {
    let manifest = agent_store_schema_manifest_payload();
    let schema_digest = digest_payload(&manifest).map_err(|error| DbError::Init(error.to_string()))?;
    let seed_digest = digest_payload(&official_preset_seed_manifest_payload()).map_err(|error| DbError::Init(error.to_string()))?;
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES ('canonical',?,'main-sqlite-agent-store',?,?,?,?) ON CONFLICT(singleton_key) DO NOTHING")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
        .bind(seed_digest.as_ref()).bind(schema_digest.as_ref()).bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION))
        .execute(&mut *conn).await?;
    let row: (i64, i64, String, i64) = sqlx::query_as("SELECT data_generation,migration_head,canonical_schema_manifest_digest,projection_schema_version FROM schema_metadata WHERE singleton_key='canonical'")
        .fetch_one(&mut *conn).await?;
    if row != (i64::from(AGENT_STORE_DATA_GENERATION), i64::from(AGENT_STORE_MIGRATION_HEAD), schema_digest.0, i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)) {
        return Err(DbError::Init("canonical schema metadata differs from this data generation".into()));
    }
    Ok(())
}

pub(crate) async fn clean_cut_agent_store(conn: &mut SqliteConnection, migrator: &Migrator) -> Result<(), DbError> {
    let present: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='_sqlx_migrations'")
        .fetch_one(&mut *conn).await?;
    if present == 0 { return Ok(()); }
    let current = migrator.iter().next().ok_or_else(|| DbError::Init("canonical baseline is missing".into()))?;
    let rows = sqlx::query("SELECT version,success,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&mut *conn).await?;
    let known = migrator.iter().collect::<Vec<_>>();
    if !rows.is_empty() && rows.len() <= known.len() && rows.iter().enumerate().all(|(index,row)| {
        let migration = known[index];
        row.try_get::<i64,_>("version").ok() == Some(migration.version)
            && row.try_get::<bool,_>("success").ok() == Some(true)
            && row.try_get::<Vec<u8>,_>("checksum").ok()
                .is_some_and(|checksum| checksum.as_slice() == migration.checksum.as_ref())
    }) {
        return Ok(());
    }
    let generation: Option<i64> = sqlx::query_scalar("SELECT data_generation FROM schema_metadata WHERE singleton_key='canonical'")
        .fetch_optional(&mut *conn).await?;
    if !exact_retired_receipts(&rows, generation.unwrap_or(0)) {
        return Err(DbError::Init("database lineage is neither current nor the complete retired Agent generation".into()));
    }
    let mut template = SqliteConnection::connect_with(&SqliteConnectOptions::new().in_memory(true)).await?;
    sqlx::raw_sql(AGENT_STORE_BASELINE_SQL).execute(&mut template).await?;
    let definitions: Vec<(String,String,String,String)> = sqlx::query_as("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 ELSE 2 END,name")
        .fetch_all(&mut template).await?;
    template.close().await?;
    let owned = RESET_ORDER.iter().map(|table| (*table).to_owned()).collect::<BTreeSet<_>>();
    let replacements = definitions.iter().filter(|(_,name,table,_)| owned.contains(table) || name == "schema_metadata").cloned().collect::<Vec<_>>();
    let mut tx = Transaction::begin(conn, Some("BEGIN IMMEDIATE".into())).await?;
    // Recheck under the writer lock: the pre-open probe grants no stale reset.
    let locked_rows = sqlx::query("SELECT version,success,checksum FROM _sqlx_migrations ORDER BY version").fetch_all(&mut *tx).await?;
    let locked_generation: i64 = sqlx::query_scalar("SELECT data_generation FROM schema_metadata WHERE singleton_key='canonical'").fetch_one(&mut *tx).await?;
    if !exact_retired_receipts(&locked_rows, locked_generation) { return Err(DbError::Conflict("Agent clean-cut authority changed before the transaction".into())); }
    verify_preserved_schema(&mut tx, &definitions, &owned).await?;
    reset_agent_data_in_transaction(&mut tx).await?;
    for table in RESET_ORDER {
        sqlx::query(&format!("DROP TABLE {table}")).execute(&mut *tx).await?;
    }
    sqlx::query("DROP TABLE schema_metadata").execute(&mut *tx).await?;
    for (_,_,_,sql) in replacements { sqlx::raw_sql(&sql).execute(&mut *tx).await?; }
    ensure_schema_metadata(&mut tx).await?;
    let violations = sqlx::query("PRAGMA foreign_key_check").fetch_all(&mut *tx).await?;
    if !violations.is_empty() { return Err(DbError::Init("Agent clean cut produced a foreign-key violation".into())); }
    sqlx::query("DELETE FROM _sqlx_migrations").execute(&mut *tx).await?;
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (?,?,1,?,0)")
        .bind(current.version).bind(current.description.as_ref()).bind(current.checksum.as_ref())
        .execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::info!("Agent-only clean cut installed the new empty canonical generation; non-Agent schema and data remain in place");
    Ok(())
}

async fn verify_preserved_schema(tx: &mut Transaction<'_,sqlx::Sqlite>, expected: &[(String,String,String,String)], owned: &BTreeSet<String>) -> Result<(), DbError> {
    let observed: Vec<(String,String,String,String)> = sqlx::query_as("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'")
        .fetch_all(&mut **tx).await?;
    let normalized = |rows: &[(String,String,String,String)]| rows.iter()
        .filter(|(_,name,table,_)| !owned.contains(table) && name != "schema_metadata")
        .map(|(kind,name,_,sql)| ((kind.clone(),name.clone()),sql.split_whitespace().collect::<String>()))
        .collect::<BTreeMap<_,_>>();
    if normalized(&observed) != normalized(expected) {
        return Err(DbError::Init("retired generation non-Agent schema differs from the current preserved schema; no reset was performed".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    struct TestDatabase { pool: SqlitePool }
    impl TestDatabase { fn pool(&self) -> &SqlitePool { &self.pool } }

    static MIGRATOR: Migrator = sqlx::migrate!();
    const SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000211";

    async fn retired_generation_fixture() -> TestDatabase {
        // The retired lineage predates forward product migrations. Build its
        // exact preserved baseline rather than downgrading a current database.
        let database = TestDatabase { pool: SqlitePoolOptions::new().max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true).foreign_keys(true)
                .pragma("secure_delete", "ON")).await.unwrap() };
        let pool = database.pool();
        sqlx::raw_sql(AGENT_STORE_BASELINE_SQL).execute(pool).await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        ensure_schema_metadata(&mut conn).await.unwrap();
        drop(conn);
        sqlx::raw_sql("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL);
            INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES ('0190f5fe-7c00-7a00-8000-000000000214','admin','',1,1);
            INSERT INTO installation_identity(singleton_key,owner_user_id) VALUES ('installation','0190f5fe-7c00-7a00-8000-000000000214');")
            .execute(pool).await.unwrap();
        let row: (String,String,String,i64) = sqlx::query_as("SELECT root_instance_id,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version FROM schema_metadata")
            .fetch_one(pool).await.unwrap();
        let ddl: String = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name='schema_metadata'")
            .fetch_one(pool).await.unwrap();
        sqlx::query("DROP TABLE schema_metadata").execute(pool).await.unwrap();
        sqlx::raw_sql(&ddl.replace("data_generation = 7", "data_generation = 6"))
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO schema_metadata VALUES ('canonical',6,?,7,?,?,?)")
            .bind(row.0).bind(row.1).bind(row.2).bind(row.3).execute(pool).await.unwrap();
        sqlx::query("DELETE FROM _sqlx_migrations").execute(pool).await.unwrap();
        for (index, checksum) in RETIRED_CHECKSUMS.iter().enumerate() {
            sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (?,'retired generation fixture',1,?,0)")
                .bind(index as i64 + 1).bind(hex::decode(checksum).unwrap()).execute(pool).await.unwrap();
        }
        // Disposable obsolete columns exercise physical replacement. The
        // product does not carry a retired schema or decode their values.
        sqlx::raw_sql("ALTER TABLE agent_sessions ADD COLUMN reasoning_effort_v2 TEXT; ALTER TABLE agent_events ADD COLUMN runtime_binding_id TEXT; ALTER TABLE agent_session_heads ADD COLUMN runtime_checkpoint_locator TEXT;")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at,reasoning_effort_v2) VALUES (?,'{}','live',0,0,'{}',1,1,'ultra')")
            .bind(SESSION).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES ('cutover-preserve','user preference',1)")
            .execute(pool).await.unwrap();
        for key in ["guid.defaultAgentSelection", "guid.agentSelection"] {
            sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES (?, '{\"kind\":\"preset\",\"presetId\":\"0190f5fe-7c00-7a00-8000-000000000299\"}',1)")
                .bind(key).execute(pool).await.unwrap();
        }
        sqlx::query("INSERT INTO providers(provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,created_at,updated_at) VALUES ('0190f5fe-7c00-7a00-8000-000000000212','openai','User provider','https://provider.example','bearer','encrypted user credential',1,1,1)")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO knowledge_bases(knowledge_base_id,name,root_path,created_at,updated_at) VALUES ('0190f5fe-7c00-7a00-8000-000000000213','User knowledge','user knowledge root',1,1)")
            .execute(pool).await.unwrap();
        database
    }

    #[tokio::test]
    async fn exact_generation_clean_cut_keeps_non_agent_data_and_replaces_agent_schema() {
        let database = retired_generation_fixture().await;
        let pool = database.pool();
        let owner_before = crate::installation_owner_id(pool).await.unwrap();
        assert!(requires_agent_store_clean_cut(pool).await.unwrap());
        let mut conn = pool.acquire().await.unwrap();
        clean_cut_agent_store(&mut conn, &MIGRATOR).await.unwrap();
        drop(conn);
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *conn).await.unwrap();
        MIGRATOR.run(&mut *conn).await.unwrap();
        sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await.unwrap();
        drop(conn);
        assert!(!requires_agent_store_clean_cut(pool).await.unwrap());
        crate::validate_current_migration_lineage(pool).await.unwrap();
        crate::validate_id_schema_contract(pool).await.unwrap();
        crate::validate_id_data_contract(pool).await.unwrap();
        assert_eq!(crate::installation_owner_id(pool).await.unwrap(), owner_before);
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(pool).await.unwrap();
        assert_eq!(rows, 0, "no retired Session or value was imported");
        let removed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('agent_sessions') WHERE name='reasoning_effort_v2'")
            .fetch_one(pool).await.unwrap();
        assert_eq!(removed, 0);
        for (table, column) in [("agent_events", "runtime_binding_id"), ("agent_session_heads", "runtime_checkpoint_locator")] {
            let columns: Vec<String> = sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .fetch_all(pool).await.unwrap();
            assert!(!columns.iter().any(|name| name == column));
        }
        let preference: String = sqlx::query_scalar("SELECT value FROM client_preferences WHERE key='cutover-preserve'")
            .fetch_one(pool).await.unwrap();
        let credential: String = sqlx::query_scalar("SELECT credentials_encrypted FROM providers WHERE name='User provider'")
            .fetch_one(pool).await.unwrap();
        let knowledge: String = sqlx::query_scalar("SELECT root_path FROM knowledge_bases WHERE name='User knowledge'")
            .fetch_one(pool).await.unwrap();
        assert_eq!(preference, "user preference");
        let old_choices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key IN ('guid.defaultAgentSelection','guid.agentSelection')")
            .fetch_one(pool).await.unwrap();
        assert_eq!(old_choices, 0);
        assert_eq!(credential, "encrypted user credential");
        assert_eq!(knowledge, "user knowledge root");
        // A repeated startup accepts only the new baseline and cannot reset
        // Sessions created in the current generation.
        sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',1,1)")
            .bind(SESSION).execute(pool).await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        clean_cut_agent_store(&mut conn, &MIGRATOR).await.unwrap();
        drop(conn);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_sessions").fetch_one(pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn partial_or_changed_receipts_never_authorize_cutover() {
        for partial in [true, false] {
            let database = retired_generation_fixture().await;
            let pool = database.pool();
            if partial {
                sqlx::query("DELETE FROM _sqlx_migrations WHERE version=11").execute(pool).await.unwrap();
            } else {
                sqlx::query("UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=11").execute(pool).await.unwrap();
            }
            assert!(!requires_agent_store_clean_cut(pool).await.unwrap());
            let mut conn = pool.acquire().await.unwrap();
            assert!(clean_cut_agent_store(&mut conn, &MIGRATOR).await.is_err());
            drop(conn);
            assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_sessions").fetch_one(pool).await.unwrap(), 1);
        }
    }

    #[tokio::test]
    async fn current_forward_receipts_reject_unknown_changed_and_gapped_lineage_without_deletion() {
        for tampering in [
            "UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=2",
            "DELETE FROM _sqlx_migrations WHERE version=1",
            "INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (3,'unknown',1,X'00',0)",
        ] {
            let database = crate::init_database_memory().await.unwrap();
            let pool = database.pool();
            sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',1,1)")
                .bind(SESSION).execute(pool).await.unwrap();
            sqlx::query(tampering).execute(pool).await.unwrap();
            let mut conn = pool.acquire().await.unwrap();
            assert!(clean_cut_agent_store(&mut conn, &MIGRATOR).await.is_err(), "{tampering}");
            drop(conn);
            assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_sessions")
                .fetch_one(pool).await.unwrap(), 1);
        }
    }

    #[tokio::test]
    async fn deletion_failure_rolls_back_rows_schema_and_lineage_together() {
        let database = retired_generation_fixture().await;
        let pool = database.pool();
        sqlx::query("CREATE TRIGGER reject_test_cutover BEFORE DELETE ON agent_sessions BEGIN SELECT RAISE(ABORT,'fixture deletion failure'); END")
            .execute(pool).await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        assert!(clean_cut_agent_store(&mut conn, &MIGRATOR).await.is_err());
        drop(conn);
        assert!(requires_agent_store_clean_cut(pool).await.unwrap());
        let old_value: String = sqlx::query_scalar("SELECT reasoning_effort_v2 FROM agent_sessions WHERE agent_session_id=?")
            .bind(SESSION).fetch_one(pool).await.unwrap();
        assert_eq!(old_value, "ultra");
        let generations: i64 = sqlx::query_scalar("SELECT data_generation FROM schema_metadata").fetch_one(pool).await.unwrap();
        assert_eq!(generations, 6);
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations").fetch_one(pool).await.unwrap();
        assert_eq!(receipts, 11);
    }
}
