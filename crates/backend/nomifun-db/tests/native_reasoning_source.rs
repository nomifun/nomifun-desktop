use nomifun_db::init_database_memory;

#[tokio::test]
async fn native_reasoning_has_one_physical_column_and_exact_tiers() {
    let database = init_database_memory().await.unwrap();
    let pool = database.pool();
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('agent_sessions') WHERE name LIKE 'reasoning_effort%'")
        .fetch_all(pool).await.unwrap();
    assert_eq!(columns, ["reasoning_effort"]);
    let session = "0190f5fe-7c00-7a00-8000-000000000211";
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',1,1)")
        .bind(session).execute(pool).await.unwrap();
    for tier in ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"] {
        sqlx::query("UPDATE agent_sessions SET reasoning_effort=? WHERE agent_session_id=?")
            .bind(tier).bind(session).execute(pool).await.unwrap();
    }
    assert!(sqlx::query("UPDATE agent_sessions SET reasoning_effort='unsupported' WHERE agent_session_id=?")
        .bind(session).execute(pool).await.is_err());
    assert!(sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,deleted_at,reasoning_effort) VALUES (?,'{}','deleted',1,'high')")
        .bind("0190f5fe-7c00-7a00-8000-000000000212").execute(pool).await.is_err());
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,deleted_at) VALUES (?,'{}','deleted',1)")
        .bind("0190f5fe-7c00-7a00-8000-000000000212").execute(pool).await.unwrap();
}
