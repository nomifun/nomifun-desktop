use std::{borrow::Cow, collections::BTreeMap, path::Path};

use nomifun_db::{MigrationLineage, inspect_migration_lineage};
use sqlx::{Row, SqlitePool, migrate::Migrator, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

static MIGRATOR: Migrator = sqlx::migrate!();
const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000091";

async fn previous_database(path: &Path) -> SqlitePool {
    let pool=SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(true),
    ).await.unwrap();
    let previous=Migrator {migrations:Cow::Owned(MIGRATOR.iter()
        .take_while(|migration|migration.version<15).cloned().collect()),..Migrator::DEFAULT};
    previous.run(&pool).await.unwrap();
    let manifest=nomifun_agent_contracts::agent_store_schema_manifest_payload();
    // This fixture's schema predates migration 016, so its declared lineage is
    // the pre-016 manifest digest, not the current manifest.
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(manifest.data_generation)).bind(i64::from(manifest.migration_head))
        .bind(nomifun_agent_contracts::digest_payload(&nomifun_agent_contracts::official_preset_seed_manifest_payload()).unwrap().as_ref())
        .bind("ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9")
        .bind(i64::from(manifest.projection_schema_version)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES(?,'migration-fixture','fixture-only',1,1)")
        .bind(OWNER).execute(&pool).await.unwrap();
    for (id,gate,state) in [
        ("0190f5fe-7c00-7a00-8000-000000000092","automatic","running"),
        ("0190f5fe-7c00-7a00-8000-000000000093","require_approval","completed"),
    ] {
        sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,max_parallel,initial_plan_input,summary,version,event_sequence,created_at,updated_at) VALUES(?,?,?, ?,?,'adaptive','ask_user','prefer_parallel',7,?,?,3,1,1,2)")
            .bind(id).bind(OWNER).bind("Preserve exact execution bytes.\n中文")
            .bind(state).bind(gate).bind("{\"mode\":\"automatic\", \"source\":null}")
            .bind("Original result\nwith final LF.\n").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'created','user',?,?,1)")
            .bind(id).bind(OWNER).bind("{\"status\":\"planning\", \"fixture\":\"保留\"}")
            .execute(&pool).await.unwrap();
    }
    let session="0190f5fe-7c00-7a00-8000-000000000094";
    let opening="0190f5fe-7c00-7a00-8000-000000000095";
    let binding=serde_json::json!({"preset_revision_ref":{"preset_id":"0190f5fe-7c00-7a00-8000-000000000096","revision":1,"revision_digest":"a".repeat(64)},
        "resolved_snapshot_ref":{"snapshot_id":"0190f5fe-7c00-7a00-8000-000000000097","snapshot_digest":"b".repeat(64)},
        "typed_resource_bindings":[],"binding_version":1});
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','Preserved history',0,0,?,2,1)")
        .bind(session).bind(serde_json::json!({"principal_kind":"user","principal_id":OWNER}).to_string())
        .bind(binding.to_string())
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'session_api','preserved-opening','session/opening',1,?,?)")
        .bind(session).bind(opening).bind(opening).bind("{\"fixture\":\"canonical bytes\", \"text\":\"中文\\n\"}")
        .execute(&pool).await.unwrap();
    sqlx::query("UPDATE system_settings SET language='zh-CN',notification_enabled=0,updated_at=99")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES('theme','\"dark\"',9)")
        .execute(&pool).await.unwrap();
    pool
}

fn identifier(value:&str)->String {format!("\"{}\"",value.replace('"',"\"\""))}

/// SQL quote retains type and bytes, including blobs and line endings. Read
/// every published data column except the one explicitly being retired.
async fn data_snapshot(pool:&SqlitePool)->BTreeMap<String,(Vec<String>,Vec<Vec<String>>)> {
    let tables:Vec<String>=sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name<>'_sqlx_migrations' ORDER BY name")
        .fetch_all(pool).await.unwrap();
    let mut snapshot=BTreeMap::new();
    for table in tables {
        let columns:Vec<String>=sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
            .bind(&table).fetch_all(pool).await.unwrap();
        let columns=columns.into_iter().filter(|column|table!="agent_executions"||column!="plan_gate").collect::<Vec<_>>();
        let query=format!("SELECT {} FROM {}",columns.iter().map(|column|format!("quote({})",identifier(column))).collect::<Vec<_>>().join(","),identifier(&table));
        let mut rows=sqlx::query(&query).fetch_all(pool).await.unwrap().into_iter().map(|row|
            (0..columns.len()).map(|index|row.get::<String,_>(index)).collect::<Vec<_>>()).collect::<Vec<_>>();
        rows.sort();snapshot.insert(table,(columns,rows));
    }
    snapshot
}

#[tokio::test]
async fn forward_015_only_removes_the_unused_column_and_preserves_every_other_value_on_reopen() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("execution-plan-gate.sqlite");
    let old=previous_database(&path).await;
    assert_eq!(inspect_migration_lineage(&old).await.unwrap(),MigrationLineage::KnownPrefix);
    let data=data_snapshot(&old).await;
    let schema:Vec<(String,String)>=sqlx::query_as("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'agent_executions' ORDER BY name")
        .fetch_all(&old).await.unwrap();
    let receipts:Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&old).await.unwrap();
    // Only migration 015 is under test: later product migrations legitimately
    // add their own tables, so data and schema comparisons span just 015.
    Migrator {migrations:Cow::Owned(MIGRATOR.iter()
        .take_while(|migration|migration.version<=15).cloned().collect()),..Migrator::DEFAULT}
        .run(&old).await.unwrap();
    assert_eq!(data_snapshot(&old).await,data);
    assert_eq!(sqlx::query_as::<_,(String,String)>("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'agent_executions' ORDER BY name")
        .fetch_all(&old).await.unwrap(),schema);
    old.close().await;
    for _ in 0..2 {
        let current=SqlitePoolOptions::new().max_connections(1).connect_with(
            SqliteConnectOptions::new().filename(&path).foreign_keys(true),
        ).await.unwrap();
        MIGRATOR.run(&current).await.unwrap();
        assert_eq!(inspect_migration_lineage(&current).await.unwrap(),MigrationLineage::Current);
        let columns:Vec<String>=sqlx::query_scalar("SELECT name FROM pragma_table_info('agent_executions')")
            .fetch_all(&current).await.unwrap();
        assert!(!columns.iter().any(|column|column=="plan_gate"));
        // Later migrations added their own product tables; the originally
        // snapshotted tables must survive the full upgrade unchanged. The
        // schema_metadata row may only advance the declared manifest digest.
        let current_digest:String=sqlx::query_scalar("SELECT canonical_schema_manifest_digest FROM schema_metadata")
            .fetch_one(&current).await.unwrap();
        assert_eq!(current_digest,nomifun_agent_contracts::digest_payload(&nomifun_agent_contracts::agent_store_schema_manifest_payload()).unwrap().as_ref());
        let current_data=data_snapshot(&current).await;
        for (table,expected) in &data {
            let actual=current_data.get(table);
            if table=="schema_metadata" {
                let mut advanced=expected.clone();
                for row in &mut advanced.1 {
                    for value in row.iter_mut() {
                        *value=value.replace("ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9",&current_digest);
                    }
                }
                assert_eq!(actual,Some(&advanced),"schema_metadata may only advance the declared manifest digest");
            } else {
                assert_eq!(actual,Some(expected),"{table} must survive the upgrade unchanged");
            }
        }
        let now:Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&current).await.unwrap();
        assert_eq!(&now[..receipts.len()],receipts.as_slice());
        assert_eq!(now,MIGRATOR.iter().map(|migration|(migration.version,migration.checksum.to_vec())).collect::<Vec<_>>());
        current.close().await;
    }
}

#[tokio::test]
async fn unknown_lineage_does_not_receive_column_retirement_authority() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("unknown-plan-gate.sqlite");
    let old=previous_database(&path).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=14").execute(&old).await.unwrap();
    assert!(MIGRATOR.run(&old).await.is_err());
    let columns:Vec<String>=sqlx::query_scalar("SELECT name FROM pragma_table_info('agent_executions')")
        .fetch_all(&old).await.unwrap();
    assert!(columns.iter().any(|column|column=="plan_gate"));
    old.close().await;
}
