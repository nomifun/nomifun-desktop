//! Exact 016 -> 017 non-Agent convergence. Only temporary databases are used.
use sha2::{Digest,Sha256};
use sqlx::{SqlitePool,sqlite::{SqliteConnectOptions,SqlitePoolOptions}};
use serde_json::{Value,json};
use std::path::Path;
const OLD_DIGEST:&str="0ef8e3eeec9835238c865441913383bb80880bf89e3f0d78b538780c7534bdfc";
fn id(number:u64)->String{format!("0190f5fe-7c00-7a00-8000-{number:012x}")}
fn task(client:&str,assignee:&str)->Value{json!({"kind":"task_create","client_item_id":client,"title":client,"assignee_id":assignee,"assignee_name":assignee,"due":"2026-10-28","tasklist_id":null})}
fn key(parts:Value)->String{format!("{:x}",Sha256::digest(serde_json::to_vec(&parts).unwrap()))}
async fn old(path:&Path)->SqlitePool{
    let pool=SqlitePoolOptions::new().max_connections(1).connect_with(SqliteConnectOptions::new().filename(path).create_if_missing(true)).await.unwrap();
    sqlx::raw_sql("CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY,description TEXT NOT NULL,installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,success BOOLEAN NOT NULL,checksum BLOB NOT NULL,execution_time BIGINT NOT NULL)").execute(&pool).await.unwrap();
    let migrator=sqlx::migrate!("./migrations");
    for m in migrator.iter().filter(|m|m.version<=16){
        sqlx::raw_sql(&m.sql).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (?,?,1,?,0)").bind(m.version).bind(&m.description).bind(m.checksum.as_ref()).execute(&pool).await.unwrap();
    }
    let seed=nomifun_agent_contracts::digest_payload(&nomifun_agent_contracts::official_preset_seed_manifest_payload()).unwrap();
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES ('canonical',7,'main-sqlite-agent-store',1,?,?,1)").bind(seed.as_ref()).bind(OLD_DIGEST).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES (?,'compact-test-owner','',1,1)").bind(id(1)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO installation_identity(singleton_key,owner_user_id) VALUES ('installation',?)").bind(id(1)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO office_connections(connection_id,owner_id,name,enabled,profile,created_at,updated_at) VALUES (?,?,'Feishu',1,'nomifun',1,1)").bind(id(2)).bind(id(1)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',1,1)").bind(id(3)).execute(&pool).await.unwrap();
    let bytes="canonical正文\n\0byte-for-byte".as_bytes();
    sqlx::query("INSERT INTO agent_payloads(payload_id,session_id,media_type,byte_len,digest,body) VALUES ('compact-payload',?,'text/plain',?,?,?)").bind(id(3)).bind(bytes.len() as i64).bind(format!("{:x}",Sha256::digest(bytes))).bind(bytes).execute(&pool).await.unwrap();
    pool
}
async fn operation(pool:&SqlitePool,n:u64,status:&str){
    sqlx::query("INSERT INTO office_operations(operation_id,owner_id,connection_id,request_id,actor_json,profile,digest,status,required_permissions_json,created_at,updated_at) VALUES (?,?,?,?,?,'nomifun',?,?,'[]',1,1)")
        .bind(id(n)).bind(id(1)).bind(id(2)).bind(id(n+1000)).bind(json!({"app_id":"cli_old","user_id":"ou_owner","name":"Old display"}).to_string()).bind("a".repeat(64)).bind(status).execute(pool).await.unwrap();
}
async fn item(pool:&SqlitePool,n:u64,op:u64,status:&str,mutation:&Value,target:&str,admitted:bool){
    sqlx::query("INSERT INTO office_operation_items(item_id,operation_id,client_item_id,ordinal,mutation_json,target_key,admitted,status,retryable) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(id(n)).bind(id(op)).bind(mutation["client_item_id"].as_str().unwrap()).bind(n as i64).bind(mutation.to_string()).bind(target).bind(admitted).bind(status).bind(status=="not_executed").execute(pool).await.unwrap();
}
async fn attempt(pool:&SqlitePool,n:u64,op:u64,item:u64,number:i64,status:&str,mutation:&Value){
    sqlx::query("INSERT INTO office_operation_attempts(attempt_id,operation_id,item_id,number,client_token,mutation_json,status,started_at,finished_at,message,result_json) VALUES (?,?,?,?,?,?,?,1,?,'original immutable outcome',?)")
        .bind(id(n)).bind(id(op)).bind(id(item)).bind(number).bind(id(n+1000)).bind(serde_json::to_string_pretty(mutation).unwrap()).bind(status).bind(if status=="running"{None}else{Some(2i64)}).bind("{\n  \"original\": true\n}").execute(pool).await.unwrap();
}

#[tokio::test]
async fn compact_upgrade_preserves_all_submitted_evidence_pending_fences_and_sequence_highwater(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("old-16.db");let pool=old(&path).await;
    operation(&pool,10,"partial").await;
    for (n,status) in [(20,"succeeded"),(21,"unknown"),(23,"not_executed")]{
        let mutation=task(&format!("client-{n}"),"ou_owner");item(&pool,n,10,status,&mutation,&key(json!(["creation",n])),true).await;attempt(&pool,n+100,10,n,1,status,&mutation).await;
    }
    operation(&pool,14,"accepted_pending_confirmation").await;
    let base=json!({"kind":"base_update","client_item_id":"base-accepted","base_token":"base-real","table_id":"table-real","record_id":"record-real","fields":{"fldStatus":["Done"]},"before":{"fldStatus":["Open"]}});
    item(&pool,22,14,"accepted_pending_confirmation",&base,&key(json!(["feishu","base","base-real","table-real","record-real"])),true).await;
    attempt(&pool,122,14,22,1,"accepted_pending_confirmation",&base).await;
    // A real retry retained both the original failed input and the new input.
    let retry=task("retry","ou_resolved");item(&pool,24,10,"succeeded",&retry,&key(json!(["creation",24])),true).await;
    attempt(&pool,124,10,24,1,"not_executed",&task("retry","unresolved")).await;attempt(&pool,125,10,24,2,"succeeded",&retry).await;
    sqlx::query("INSERT INTO office_operation_observations(observation_id,operation_id,item_id,kind,message,data_json,confirmed,created_at) VALUES (?,?,?,'original_receipt','exact late receipt',?,1,3)")
        .bind(id(200)).bind(id(10)).bind(id(24)).bind(json!({"attempt_id":id(125),"guid":"real-guid"}).to_string()).execute(&pool).await.unwrap();
    operation(&pool,11,"running").await;let running=task("running","ou_owner");let queued=task("queued","ou_owner");
    item(&pool,25,11,"running",&running,&key(json!(["creation",25])),true).await;attempt(&pool,126,11,25,1,"running",&running).await;
    item(&pool,26,11,"queued",&queued,&key(json!(["creation",26])),true).await;
    operation(&pool,12,"unknown").await;
    let legacy=json!({"kind":"document_update","client_item_id":"legacy-doc","document_id":"doc-real","revision":"12","content":"old append input"});
    let doc_key=key(json!(["feishu","document","doc-real"]));item(&pool,27,12,"unknown",&legacy,&doc_key,true).await;attempt(&pool,127,12,27,1,"unknown",&legacy).await;
    operation(&pool,13,"draft").await;item(&pool,28,13,"queued",&task("never-dispatched","ou_a"),&key(json!(["draft"])),false).await;
    sqlx::query("UPDATE sqlite_sequence SET seq=12000 WHERE name IN ('office_operations','office_operation_items')").execute(&pool).await.unwrap();
    let rich=json!({"id":"doc-real","kind":"document","title":"Actual opened entrance","url":"https://test.invalid/docx/doc-real","fields":{"private":"not a bookmark"},"description":"do not migrate remote contents","pending_operation_id":id(12),"base_token":null,"table_id":null});
    sqlx::query("INSERT INTO office_recent_resources(recent_resource_id,owner_id,connection_id,resource_key,resource_json,last_opened_at) VALUES (?,?,?,?,?,99)").bind(id(201)).bind(id(1)).bind(id(2)).bind("b".repeat(64)).bind(rich.to_string()).execute(&pool).await.unwrap();
    let before:Vec<(String,String,Option<String>)>=sqlx::query_as("SELECT attempt_id,mutation_json,result_json FROM office_operation_attempts ORDER BY attempt_id").fetch_all(&pool).await.unwrap();
    let checksums:Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version").fetch_all(&pool).await.unwrap();
    let bytes:Vec<u8>=sqlx::query_scalar("SELECT body FROM agent_payloads WHERE payload_id='compact-payload'").fetch_one(&pool).await.unwrap();pool.close().await;
    let upgraded=nomifun_db::init_database(&path).await.unwrap();let pool=upgraded.pool();
    let after:Vec<(String,String,Option<String>)>=sqlx::query_as("SELECT receipt_id,mutation_json,data_json FROM office_operation_receipts WHERE record_kind='attempt' ORDER BY receipt_id").fetch_all(pool).await.unwrap();assert_eq!(before,after);
    assert_eq!(checksums,sqlx::query_as::<_,(i64,Vec<u8>)>("SELECT version,checksum FROM _sqlx_migrations WHERE version<=16 ORDER BY version").fetch_all(pool).await.unwrap());
    assert_eq!(bytes,sqlx::query_scalar::<_,Vec<u8>>("SELECT body FROM agent_payloads WHERE payload_id='compact-payload'").fetch_one(pool).await.unwrap());
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM office_operations").fetch_one(pool).await.unwrap(),4);
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT status FROM office_operation_items WHERE item_id=?").bind(id(25)).fetch_one(pool).await.unwrap(),"running");
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT status FROM office_operation_items WHERE item_id=?").bind(id(26)).fetch_one(pool).await.unwrap(),"queued");
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT target_key FROM office_operation_items WHERE item_id=?").bind(id(27)).fetch_one(pool).await.unwrap(),doc_key);
    for table in ["office_operations","office_operation_items"]{assert_eq!(sqlx::query_scalar::<_,i64>("SELECT seq FROM sqlite_sequence WHERE name=?").bind(table).fetch_one(pool).await.unwrap(),12000)}
    let observation:String=sqlx::query_scalar("SELECT data_json FROM office_operation_receipts WHERE receipt_id=?").bind(id(200)).fetch_one(pool).await.unwrap();let observation:Value=serde_json::from_str(&observation).unwrap();assert_eq!(observation["confirmed"],true);assert_eq!(observation["kind"],"original_receipt");assert_eq!(observation["data"]["attempt_id"],id(125));
    let navigation:String=sqlx::query_scalar("SELECT recent_resources_json FROM office_connections").fetch_one(pool).await.unwrap();assert!(navigation.contains("Actual opened entrance"));assert!(!navigation.contains("private"));assert!(!navigation.contains("description"));assert!(!navigation.contains("pending_operation_id"));assert_eq!(serde_json::from_str::<Value>(&navigation).unwrap()[0]["opened_at"],99);
    nomifun_db::validate_id_schema_contract(pool).await.unwrap();nomifun_db::validate_id_data_contract(pool).await.unwrap();
    let reset=nomifun_db::reset_agent_data(pool).await.unwrap();for table in ["office_connections","office_operations","office_operation_items","office_operation_receipts"]{assert!(reset.preserved_rows.contains_key(table))}
    let snapshot=dir.path().join("snapshot.db");upgraded.snapshot_into(&snapshot).await.unwrap();let restored=nomifun_db::open_database_for_backup(&snapshot).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT mutation_json FROM office_operation_items WHERE item_id=?").bind(id(27)).fetch_one(restored.pool()).await.unwrap(),legacy.to_string());
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT recent_resources_json FROM office_connections").fetch_one(restored.pool()).await.unwrap(),navigation);
    restored.close().await;upgraded.close().await;
}

#[tokio::test]
async fn compact_upgrade_refuses_a_draft_with_execution_facts_without_changing_its_source(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("contradictory-draft.db");let pool=old(&path).await;
    operation(&pool,10,"draft").await;let mutation=task("draft","ou_a");item(&pool,20,10,"queued",&mutation,&key(json!(["draft"])),false).await;attempt(&pool,120,10,20,1,"unknown",&mutation).await;pool.close().await;
    assert!(nomifun_db::init_database(&path).await.is_err());
    let pool=SqlitePoolOptions::new().max_connections(1).connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(false)).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM office_operation_attempts").fetch_one(&pool).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=17").fetch_one(&pool).await.unwrap(),0);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM sqlite_schema WHERE name='office_operation_receipts'").fetch_one(&pool).await.unwrap(),0);pool.close().await;
}

#[tokio::test]
async fn compact_upgrade_rejects_unknown_metadata_without_losing_unresolved_receipts(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("unknown-metadata.db");let pool=old(&path).await;
    operation(&pool,10,"unknown").await;let mutation=task("unknown","ou_a");item(&pool,20,10,"unknown",&mutation,&key(json!(["unknown"])),true).await;attempt(&pool,120,10,20,1,"unknown",&mutation).await;
    sqlx::query("UPDATE schema_metadata SET canonical_schema_manifest_digest=?").bind("f".repeat(64)).execute(&pool).await.unwrap();pool.close().await;
    assert!(nomifun_db::init_database(&path).await.is_err());
    let pool=SqlitePoolOptions::new().max_connections(1).connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(false)).await.unwrap();assert_eq!(sqlx::query_scalar::<_,String>("SELECT status FROM office_operation_attempts").fetch_one(&pool).await.unwrap(),"unknown");assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=17").fetch_one(&pool).await.unwrap(),0);pool.close().await;
}

#[tokio::test]
async fn receipt_checks_reject_null_discriminator_fields_and_missing_observation_proof(){
    let db=nomifun_db::init_database_memory().await.unwrap();
    let bad=sqlx::query("INSERT INTO office_operation_receipts(receipt_id,item_id,record_kind,number,client_token,mutation_json,status,occurred_at) VALUES (?,?,'attempt',NULL,?,'{}','running',1)")
        .bind(id(500)).bind(id(501)).bind(id(502)).execute(db.pool()).await;
    assert!(bad.is_err(),"NULL attempt number must not pass a CHECK as unknown");
    for data in [json!({}),json!({"kind":"target_readback"}),json!({"confirmed":true}),json!({"kind":"target_readback","confirmed":0}),json!({"kind":"unsupported","confirmed":false})]{
        assert!(sqlx::query("INSERT INTO office_operation_receipts(receipt_id,item_id,record_kind,message,data_json,occurred_at) VALUES (?,?,'observation','bad shape',?,1)")
            .bind(id(503)).bind(id(501)).bind(data.to_string()).execute(db.pool()).await.is_err(),"bad observation {data}");
    }
    db.close().await;
}

#[tokio::test]
async fn compact_upgrade_rejects_malformed_navigation_instead_of_deleting_its_source(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("bad-navigation.db");let pool=old(&path).await;
    sqlx::query("INSERT INTO office_recent_resources(recent_resource_id,owner_id,connection_id,resource_key,resource_json,last_opened_at) VALUES (?,?,?,?,?,1)")
        .bind(id(600)).bind(id(1)).bind(id(2)).bind("d".repeat(64)).bind("{}").execute(&pool).await.unwrap();pool.close().await;
    assert!(nomifun_db::init_database(&path).await.is_err());
    let pool=SqlitePoolOptions::new().max_connections(1).connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(false)).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT resource_json FROM office_recent_resources").fetch_one(&pool).await.unwrap(),"{}");
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=17").fetch_one(&pool).await.unwrap(),0);pool.close().await;
}
