//! One-way migration preserves the three historical media aggregates and running jobs.
use sqlx::migrate::{Migrate, Migrator};
use sqlx::{Row, SqlitePool};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const OWNER: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const OTHER: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-000000000003";
const ASSET: &str = "0190f5fe-7c00-7a00-8abc-000000000004";

async fn migrate_to(pool: &SqlitePool, max: i64) -> Result<(), sqlx::migrate::MigrateError> {
    let mut conn = pool.acquire().await?;
    conn.ensure_migrations_table().await?;
    let applied: std::collections::BTreeSet<_> = conn.list_applied_migrations().await?
        .into_iter().map(|m| m.version).collect();
    for migration in MIGRATOR.iter().filter(|m| m.version <= max && !applied.contains(&m.version)) {
        conn.apply(migration).await?;
    }
    Ok(())
}

async fn fixture(owner: bool) -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect("sqlite::memory:").await.unwrap();
    migrate_to(&pool, 106).await.unwrap();
    // Deliberately insert an unrelated user first: task ownership must not be guessed.
    for (id, name) in [(OTHER, "other"), (OWNER, "owner")] {
        sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES (?,?,'unused',0,0)")
            .bind(id).bind(name).execute(&pool).await.unwrap();
    }
    if owner {
        sqlx::query("INSERT INTO installation_identity(singleton_key,owner_user_id) VALUES ('installation',?)")
            .bind(OWNER).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO workshop_assets(asset_id,kind,title,tags,in_library,origin,created_at,updated_at) VALUES (?,'image','result','[]',1,?,1,1)")
        .bind(ASSET).bind(serde_json::json!({"creation_task_id": task_id(0),"workbench_kind":"image","prompt":"original"}).to_string())
        .execute(&pool).await.unwrap();
    for (n, kind, status, deleted) in [(0,"image","succeeded",None), (1,"image","failed",Some(250)), (2,"video","running",None), (3,"audio","canceled",Some(250))] {
        let params = serde_json::json!({"prompt":format!("original {n}"),"seed":42,"_nomifun_connection_config_ref":format!("provider:{PROVIDER}@2"),"_nomifun_provider_config_revision":2}).to_string();
        let inputs = if n==1 { None } else { Some("[]") };
        let results = if n==0 { serde_json::json!([ASSET]).to_string() } else { "[]".into() };
        let fingerprint = serde_json::json!({"owner":{"kind":"standalone_workbench","workbench_kind":kind},"params":{"seed":42},"inputs":inputs}).to_string();
        sqlx::query("INSERT INTO creation_tasks(creation_task_id,workbench_kind,provider_id,model,capability,params,input_bindings,status,error,result_asset_ids,remote_task_id,attempt,submitted_at,started_at,finished_at,deleted_at,request_fingerprint) VALUES (?,?,?,'exact-model',?,?,?, ?,?,?,?,2,100,110,?,?,?)")
            .bind(task_id(n)).bind(kind).bind(PROVIDER)
            .bind(match kind { "image"=>"t2i","video"=>"t2v",_=>"tts" }).bind(params).bind(inputs).bind(status)
            .bind(if n==1 {Some(r#"{"kind":"provider_error","message":"original failure"}"#)} else {None})
            .bind(results).bind(if n==2 { Some("remote-job-unchanged") } else { None })
            .bind(if n==2 { None } else { Some(200i64) }).bind(deleted).bind(fingerprint)
            .execute(&pool).await.unwrap();
    }
    pool
}

fn task_id(n: i32) -> String { format!("0190f5fe-7c00-7a00-8abc-00000000001{n}") }

#[tokio::test]
async fn standalone_history_migrates_once_without_losing_inputs_results_or_remote_jobs() {
    let pool = fixture(true).await;
    sqlx::query("INSERT INTO conversations(conversation_id,user_id,name,type,created_at,updated_at) VALUES (?,?,'Unrelated existing conversation','nomi',1,1)")
        .bind("0190f5fe-7c00-7a00-8abc-000000000099").bind(OTHER).execute(&pool).await.unwrap();
    let preserved_sql = "SELECT creation_task_id,provider_id,model,capability,params,input_bindings,status,error,result_asset_ids,remote_task_id,attempt,submitted_at,started_at,finished_at,deleted_at FROM creation_tasks ORDER BY creation_task_id";
    let before = sqlx::query(preserved_sql).fetch_all(&pool).await.unwrap();
    migrate_to(&pool,107).await.unwrap();
    let after = sqlx::query(preserved_sql).fetch_all(&pool).await.unwrap();
    assert_eq!(before.len(),after.len());
    for (a,b) in before.iter().zip(&after) {
        for col in ["creation_task_id","provider_id","model","capability","params","input_bindings","status","error","result_asset_ids","remote_task_id"] {
            assert_eq!(a.get::<Option<String>,_>(col),b.get::<Option<String>,_>(col),"{col}");
        }
        for col in ["attempt","submitted_at","started_at","finished_at","deleted_at"] {
            assert_eq!(a.get::<Option<i64>,_>(col),b.get::<Option<i64>,_>(col),"{col}");
        }
    }
    let sessions: Vec<(String,String,String)> = sqlx::query_as("SELECT conversation_id,user_id,name FROM conversations WHERE json_type(extra,'$.creation_history_import') IS NOT NULL ORDER BY name").fetch_all(&pool).await.unwrap();
    assert_eq!(sessions.len(),3);
    assert!(sessions.iter().all(|(_,user,_)|user==OWNER));
    for (id,_,_) in &sessions { nomifun_common::validate_uuidv7(id).unwrap(); }
    assert!(sessions.iter().any(|(_,_,name)|name=="语音生成历史"));
    let rows=sqlx::query("SELECT t.creation_task_id,t.conversation_id,t.message_id,t.request_fingerprint,t.deleted_at,m.content,m.hidden,m.position,m.created_at FROM creation_tasks t JOIN messages m ON m.message_id=t.message_id AND m.conversation_id=t.conversation_id ORDER BY t.creation_task_id").fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(),4);
    assert_eq!(rows[0].get::<String,_>("conversation_id"),rows[1].get::<String,_>("conversation_id"));
    for (n,row) in rows.iter().enumerate() {
        let content: serde_json::Value=serde_json::from_str(row.get::<&str,_>("content")).unwrap();
        let fp: serde_json::Value=serde_json::from_str(row.get::<&str,_>("request_fingerprint")).unwrap();
        assert_eq!(content["content"],format!("original {n}"));
        assert_eq!(content["creation"]["creation_task_id"],task_id(n as i32));
        assert_eq!(fp["owner"]["kind"],"conversation_turn");
        assert_eq!(fp["owner"]["conversation_id"],row.get::<String,_>("conversation_id"));
        assert_eq!(fp["owner"]["message_id"],row.get::<String,_>("message_id"));
        assert_eq!(fp["params"]["seed"],42);
        assert_eq!(row.get::<i64,_>("hidden"),i64::from(row.get::<Option<i64>,_>("deleted_at").is_some()));
        assert_eq!(row.get::<String,_>("position"),"right");
        assert_eq!(row.get::<i64,_>("created_at"),100);
    }
    let origin: String = sqlx::query_scalar("SELECT origin FROM workshop_assets WHERE asset_id=?").bind(ASSET).fetch_one(&pool).await.unwrap();
    let origin: serde_json::Value=serde_json::from_str(&origin).unwrap();
    assert!(origin.get("workbench_kind").is_none());
    assert_eq!(origin["conversation_id"],rows[0].get::<String,_>("conversation_id"));
    assert_eq!(origin["creation_task_id"],task_id(0));
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('creation_tasks')").fetch_all(&pool).await.unwrap();
    assert!(!columns.iter().any(|c|c=="workbench_kind"));
    // sqlx records a single application; rerunning migration discovery creates no duplicate history.
    migrate_to(&pool,107).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM conversations").fetch_one(&pool).await.unwrap(),4);
}

#[tokio::test]
async fn missing_installation_owner_aborts_without_guessing_or_losing_history() {
    let pool=fixture(false).await;
    assert!(migrate_to(&pool,107).await.is_err());
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM creation_tasks WHERE workbench_kind IS NOT NULL").fetch_one(&pool).await.unwrap(),4);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM conversations").fetch_one(&pool).await.unwrap(),0);
}
