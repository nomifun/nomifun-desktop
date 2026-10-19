use super::*;

async fn payload(pool:&SqlitePool,session:&str,id:&str,media:&str,size:i64,chunks:Vec<Value>) {
    let body=json!({"encoding":"json","value":{"output_archive":{"chunks":chunks}}});
    sqlx::query("INSERT INTO agent_payloads(payload_id,session_id,media_type,byte_len,body) VALUES(?,?,?,?,?)")
        .bind(id).bind(session).bind(media).bind(size).bind(serde_json::to_vec(&body).unwrap()).execute(pool).await.unwrap();
}

async fn event(pool:&SqlitePool,session:&str,kind:&str,producer:&str,id:Option<&str>) {
    sqlx::query("INSERT INTO agent_events(session_id,kind,producer_id,payload_id) VALUES(?,?,?,?)")
        .bind(session).bind(kind).bind(producer).bind(id).execute(pool).await.unwrap();
}

#[tokio::test]
async fn many_native_roots_use_one_materialized_reference_set_without_hiding_model_or_orphan_bytes() {
    // Isolated query/provenance fixture. These minimal source rows exercise
    // only budget selection and its read plan, not native reap authorization.
    let pool=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE agent_payloads(payload_id TEXT PRIMARY KEY,session_id TEXT NOT NULL,media_type TEXT NOT NULL,byte_len INTEGER NOT NULL,body BLOB NOT NULL)",
        "CREATE INDEX payload_session ON agent_payloads(session_id)",
        "CREATE TABLE agent_events(session_id TEXT NOT NULL,kind TEXT NOT NULL,producer_id TEXT NOT NULL,payload_id TEXT)",
        "CREATE INDEX event_session_kind ON agent_events(session_id,kind)",
    ] {sqlx::query(sql).execute(&pool).await.unwrap();}
    let session:AgentSessionId="query-session".into();let sid=session.as_ref();
    for root in 0..128 {
        let mut chunks=Vec::new();
        for leaf in 0..8 {
            let id=format!("native-{root}-{leaf}");payload(&pool,sid,&id,OUTPUT_MEDIA,11,vec![]).await;chunks.push(json!(id));
        }
        if root==0 {chunks.extend([Value::Null,json!(17),json!({"not":"a reference"}),json!("ordinary-leaf")]);}
        let id=format!("root-{root}");payload(&pool,sid,&id,RECEIPT_MEDIA,17,chunks).await;
        event(&pool,sid,"session/process-reaped","session_api",Some(&id)).await;
        if root==0 {event(&pool,sid,"session/process-reaped","session_api",Some(&id)).await;}
    }
    let mut expected=0i64;
    for index in 0..32 {
        let size=index+1;payload(&pool,sid,&format!("model-{index}"),"application/json",size,vec![]).await;expected+=size;
    }
    payload(&pool,sid,"ordinary-leaf","application/json",19,vec![]).await;expected+=19;
    for (id,media,size) in [("orphan-native",OUTPUT_MEDIA,7),("orphan-root",RECEIPT_MEDIA,13)] {
        payload(&pool,sid,id,media,size,vec![]).await;expected+=size;
    }
    for (id,kind,producer,media) in [
        ("wrong-producer","session/process-reaped","runtime_supervisor",RECEIPT_MEDIA),
        ("wrong-kind","tool/result-recorded","session_api",RECEIPT_MEDIA),
        ("wrong-root-media","session/process-reaped","session_api","application/json"),
    ] {
        let leaf=format!("{id}-leaf");payload(&pool,sid,&leaf,OUTPUT_MEDIA,11,vec![]).await;
        payload(&pool,sid,id,media,17,vec![json!(leaf)]).await;event(&pool,sid,kind,producer,Some(id)).await;expected+=28;
    }
    // A foreign root cannot classify this Session's bytes, and an event cannot
    // borrow a root stored under a different Session even with its media label.
    payload(&pool,sid,"foreign-linked-leaf",OUTPUT_MEDIA,29,vec![]).await;expected+=29;
    payload(&pool,"foreign","foreign-root",RECEIPT_MEDIA,17,vec![json!("foreign-linked-leaf")]).await;
    event(&pool,"foreign","session/process-reaped","session_api",Some("foreign-root")).await;
    event(&pool,sid,"session/process-reaped","session_api",Some("foreign-root")).await;
    payload(&pool,sid,"file-observation","application/json",23,vec![]).await;expected+=23;
    event(&pool,sid,"turn/workspace-files-observed","runtime_supervisor",Some("file-observation")).await;
    event(&pool,sid,"turn/workspace-files-observed","runtime_supervisor",Some("file-observation")).await;
    event(&pool,sid,"turn/workspace-files-observed","runtime_supervisor",None).await;
    event(&pool,"foreign","turn/workspace-files-observed","runtime_supervisor",Some("ordinary-leaf")).await;

    let before:(i64,i64,i64)=sqlx::query_as("SELECT COUNT(*),SUM(byte_len),SUM(length(body)) FROM agent_payloads").fetch_one(&pool).await.unwrap();
    let events_before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events").fetch_one(&pool).await.unwrap();
    let mut tx=pool.begin().await.unwrap();
    assert_eq!(model_payload_bytes_inner_tx(&mut tx,&session,false).await.unwrap(),expected);
    assert_eq!(model_payload_bytes_inner_tx(&mut tx,&session,true).await.unwrap(),expected-23);
    tx.commit().await.unwrap();
    let explain=format!("EXPLAIN QUERY PLAN {MODEL_PAYLOAD_BYTES_SQL}");
    let plan:Vec<(i64,i64,i64,String)>=sqlx::query_as(&explain)
        .bind(sid).bind(RECEIPT_MEDIA).bind(RECEIPT_MEDIA).bind(OUTPUT_MEDIA).bind(sid).bind(false).bind(sid)
        .fetch_all(&pool).await.unwrap();
    let details:Vec<_>=plan.iter().map(|row|row.3.as_str()).collect();
    assert!(details.iter().any(|detail|detail.contains("MATERIALIZE native_roots")),"{details:?}");
    assert!(details.iter().any(|detail|detail.contains("MATERIALIZE native_archive_payloads")),"{details:?}");
    assert!(details.iter().any(|detail|detail.contains("SEARCH archive")&&detail.contains("INDEX")),"{details:?}");
    assert!(!details.iter().any(|detail|detail.contains("CORRELATED")),"archive roots must not be reevaluated for every Payload: {details:?}");
    assert_eq!(sqlx::query_as::<_,(i64,i64,i64)>("SELECT COUNT(*),SUM(byte_len),SUM(length(body)) FROM agent_payloads").fetch_one(&pool).await.unwrap(),before);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_events").fetch_one(&pool).await.unwrap(),events_before);
    pool.close().await;
}
